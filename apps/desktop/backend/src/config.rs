use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Manager};

const SETTINGS_USER_ID: &str = "local_user";
const WORKBENCH_CONFIG_KEY: &str = "desktop.workbench";
const SETTINGS_SCHEMA_VERSION_KEY: &str = "desktop.schema_version";
pub(crate) const SETTINGS_SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct KnowledgeBit {
    pub id: String,
    pub label: String,
    pub content: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CustomPersona {
    pub id: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UserConfig {
    // --- Web Search & Scraping ---
    #[serde(default = "default_search_concurrency")]
    pub search_concurrency_limit: u32,

    #[serde(default = "default_scrape_concurrency")]
    pub scrape_concurrency_limit: u32,

    #[serde(default = "default_max_search_results")]
    pub max_search_results: u32,

    #[serde(default = "default_max_scrape_chars")]
    pub max_scrape_chars: u32,

    #[serde(default = "default_scrape_timeout")]
    pub scrape_timeout_secs: u32,

    // --- LLM & Context ---
    #[serde(default = "default_context_window")]
    pub default_context_window: u32,

    #[serde(default = "default_chunk_size")]
    pub summarization_chunk_size: u32,

    #[serde(default = "default_temperature")]
    pub llm_temperature: f32,

    #[serde(default = "default_top_p")]
    pub llm_top_p: f32,

    // --- Vector Store ---
    #[serde(default = "default_vector_dims")]
    pub vector_dimensions: u32,

    // --- Image Generation ---
    #[serde(default = "default_sd_threads")]
    pub sd_threads: u32,

    // --- Personalization ---
    #[serde(default)]
    pub knowledge_bits: Vec<KnowledgeBit>,

    #[serde(default)]
    pub custom_personas: Vec<CustomPersona>,

    #[serde(default = "default_false")]
    pub image_prompt_enhance_enabled: bool,

    #[serde(default = "default_persona")]
    pub selected_persona: String,
    #[serde(default)]
    pub selected_chat_provider: Option<String>, // "local", "anthropic", "openai", "openrouter"

    #[serde(default = "default_memory_reservation")]
    pub memory_reservation_gb: u32,
    #[serde(default = "default_true")]
    pub enable_memory_reservation: bool,
    #[serde(default = "default_false")]
    pub mlock: bool,
    #[serde(default = "default_false")]
    pub quantize_kv: bool,

    #[serde(default = "default_spotlight_shortcut")]
    pub spotlight_shortcut: String,

    /// Global keyboard shortcut for push-to-talk.
    /// Press to start recording, release to stop and transcribe.
    #[serde(default = "default_ptt_shortcut")]
    pub ptt_shortcut: String,

    #[serde(default)]
    pub disabled_providers: Vec<String>,

    // --- MCP Integration ---
    /// MCP server base URL (e.g. "https://api.thinclaw.dev")
    /// Falls back to THINCLAW_MCP_URL, then legacy SCRAPPY_MCP_URL, if not set
    /// in config.
    #[serde(default = "default_mcp_base_url")]
    pub mcp_base_url: Option<String>,

    /// MCP JWT auth token. Falls back to THINCLAW_MCP_TOKEN, then legacy
    /// SCRAPPY_MCP_TOKEN.
    #[serde(default = "default_mcp_auth_token")]
    pub mcp_auth_token: Option<String>,

    /// Whether to use the Rhai sandbox (code-execution mode) instead of
    /// legacy JSON <tool_code> parsing. Requires mcp_base_url to be set.
    #[serde(default = "default_false")]
    pub mcp_sandbox_enabled: bool,

    /// How long (in seconds) the ToolRegistryCache holds entries before
    /// re-fetching from the MCP server. Default: 300 s (5 minutes).
    #[serde(default = "default_mcp_cache_ttl")]
    pub mcp_cache_ttl_secs: u32,

    /// Maximum characters returned by a single tool call before truncation.
    /// Larger values give the agent more context at the cost of token usage.
    /// Default: 5000.
    #[serde(default = "default_mcp_tool_result_max_chars")]
    pub mcp_tool_result_max_chars: u32,

    // --- Inference Backend Selection ---
    // Each modality can independently use "local" or a cloud provider id.
    // These replace the old `selected_chat_provider` for all modalities.
    /// Chat backend: "local", "anthropic", "openai", "gemini", etc.
    /// Falls back to `selected_chat_provider` for backward compat.
    #[serde(default)]
    pub chat_backend: Option<String>,

    /// Embedding backend: "local", "openai", "gemini", "voyage", "cohere".
    #[serde(default)]
    pub embedding_backend: Option<String>,

    /// TTS backend: "local", "openai", "elevenlabs", "gemini".
    #[serde(default)]
    pub tts_backend: Option<String>,

    /// STT backend: "local", "openai", "gemini", "deepgram".
    #[serde(default)]
    pub stt_backend: Option<String>,

    /// Diffusion backend: "local", "openai", "gemini", "stability", "fal", "together".
    #[serde(default)]
    pub diffusion_backend: Option<String>,

    /// Per-modality model selection (JSON object: { "chat": "gpt-4o", "embedding": "text-embedding-3-small", ... }).
    #[serde(default)]
    pub inference_models: Option<std::collections::HashMap<String, String>>,

    /// Context window size for the currently selected cloud model.
    /// Set by the frontend when a user picks a discovered model.
    /// Falls back to the provider's `default_context_size` when `None`.
    #[serde(default)]
    pub selected_model_context_size: Option<u32>,
}

/// Presence-aware patch for [`UserConfig`].
///
/// `UserConfig` fields use Serde defaults so older recovery snapshots can be
/// upgraded safely. That same behavior is wrong for a command patch: a field
/// omitted by the caller would be replaced with its default before the command
/// could tell that it was absent. `PatchField` preserves that distinction,
/// including the difference between an omitted nullable field and an explicit
/// `null` used to clear it.
#[derive(Debug, Default)]
enum PatchField<T> {
    #[default]
    Missing,
    Value(T),
}

impl<'de, T> Deserialize<'de> for PatchField<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Value)
    }
}

#[derive(Debug, Default, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct UserConfigPatch {
    #[serde(default)]
    #[specta(optional, type = u32)]
    search_concurrency_limit: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    scrape_concurrency_limit: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    max_search_results: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    max_scrape_chars: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    scrape_timeout_secs: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    default_context_window: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    summarization_chunk_size: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = f32)]
    llm_temperature: PatchField<f32>,
    #[serde(default)]
    #[specta(optional, type = f32)]
    llm_top_p: PatchField<f32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    vector_dimensions: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    sd_threads: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = Vec<KnowledgeBit>)]
    knowledge_bits: PatchField<Vec<KnowledgeBit>>,
    #[serde(default)]
    #[specta(optional, type = Vec<CustomPersona>)]
    custom_personas: PatchField<Vec<CustomPersona>>,
    #[serde(default)]
    #[specta(optional, type = bool)]
    image_prompt_enhance_enabled: PatchField<bool>,
    #[serde(default)]
    #[specta(optional, type = String)]
    selected_persona: PatchField<String>,
    #[serde(default)]
    #[specta(optional, type = Option<String>)]
    selected_chat_provider: PatchField<Option<String>>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    memory_reservation_gb: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = bool)]
    enable_memory_reservation: PatchField<bool>,
    #[serde(default)]
    #[specta(optional, type = bool)]
    mlock: PatchField<bool>,
    #[serde(default)]
    #[specta(optional, type = bool)]
    quantize_kv: PatchField<bool>,
    #[serde(default)]
    #[specta(optional, type = String)]
    spotlight_shortcut: PatchField<String>,
    #[serde(default)]
    #[specta(optional, type = String)]
    ptt_shortcut: PatchField<String>,
    #[serde(default)]
    #[specta(optional, type = Vec<String>)]
    disabled_providers: PatchField<Vec<String>>,
    #[serde(default)]
    #[specta(optional, type = Option<String>)]
    mcp_base_url: PatchField<Option<String>>,
    #[serde(default)]
    #[specta(optional, type = Option<String>)]
    mcp_auth_token: PatchField<Option<String>>,
    #[serde(default)]
    #[specta(optional, type = bool)]
    mcp_sandbox_enabled: PatchField<bool>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    mcp_cache_ttl_secs: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = u32)]
    mcp_tool_result_max_chars: PatchField<u32>,
    #[serde(default)]
    #[specta(optional, type = Option<String>)]
    chat_backend: PatchField<Option<String>>,
    #[serde(default)]
    #[specta(optional, type = Option<String>)]
    embedding_backend: PatchField<Option<String>>,
    #[serde(default)]
    #[specta(optional, type = Option<String>)]
    tts_backend: PatchField<Option<String>>,
    #[serde(default)]
    #[specta(optional, type = Option<String>)]
    stt_backend: PatchField<Option<String>>,
    #[serde(default)]
    #[specta(optional, type = Option<String>)]
    diffusion_backend: PatchField<Option<String>>,
    #[serde(default)]
    #[specta(optional, type = Option<std::collections::HashMap<String, String>>)]
    inference_models: PatchField<Option<std::collections::HashMap<String, String>>>,
    #[serde(default)]
    #[specta(optional, type = Option<u32>)]
    selected_model_context_size: PatchField<Option<u32>>,
}

impl UserConfigPatch {
    fn into_json(self) -> Result<serde_json::Value, String> {
        serde_json::to_value(self).map_err(|error| error.to_string())
    }
}

impl Serialize for UserConfigPatch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap as _;

        let mut map = serializer.serialize_map(None)?;
        macro_rules! serialize_present {
            ($($field:ident),+ $(,)?) => {
                $(
                    if let PatchField::Value(value) = &self.$field {
                        map.serialize_entry(stringify!($field), value)?;
                    }
                )+
            };
        }
        serialize_present!(
            search_concurrency_limit,
            scrape_concurrency_limit,
            max_search_results,
            max_scrape_chars,
            scrape_timeout_secs,
            default_context_window,
            summarization_chunk_size,
            llm_temperature,
            llm_top_p,
            vector_dimensions,
            sd_threads,
            knowledge_bits,
            custom_personas,
            image_prompt_enhance_enabled,
            selected_persona,
            selected_chat_provider,
            memory_reservation_gb,
            enable_memory_reservation,
            mlock,
            quantize_kv,
            spotlight_shortcut,
            ptt_shortcut,
            disabled_providers,
            mcp_base_url,
            mcp_auth_token,
            mcp_sandbox_enabled,
            mcp_cache_ttl_secs,
            mcp_tool_result_max_chars,
            chat_backend,
            embedding_backend,
            tts_backend,
            stt_backend,
            diffusion_backend,
            inference_models,
            selected_model_context_size,
        );
        map.end()
    }
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            search_concurrency_limit: default_search_concurrency(),
            scrape_concurrency_limit: default_scrape_concurrency(),
            max_search_results: default_max_search_results(),
            max_scrape_chars: default_max_scrape_chars(),
            default_context_window: default_context_window(),
            summarization_chunk_size: default_chunk_size(),
            llm_temperature: default_temperature(),
            llm_top_p: default_top_p(),
            scrape_timeout_secs: default_scrape_timeout(),
            vector_dimensions: default_vector_dims(),
            sd_threads: default_sd_threads(),
            knowledge_bits: vec![],
            custom_personas: vec![],
            image_prompt_enhance_enabled: false,
            selected_persona: default_persona(),
            selected_chat_provider: None,
            memory_reservation_gb: default_memory_reservation(),
            enable_memory_reservation: true,
            mlock: false,
            quantize_kv: false,
            spotlight_shortcut: default_spotlight_shortcut(),
            ptt_shortcut: default_ptt_shortcut(),
            disabled_providers: vec![],
            mcp_base_url: default_mcp_base_url(),
            mcp_auth_token: default_mcp_auth_token(),
            mcp_sandbox_enabled: false,
            mcp_cache_ttl_secs: default_mcp_cache_ttl(),
            mcp_tool_result_max_chars: default_mcp_tool_result_max_chars(),
            chat_backend: None,
            embedding_backend: None,
            tts_backend: None,
            stt_backend: None,
            diffusion_backend: None,
            inference_models: None,
            selected_model_context_size: None,
        }
    }
}

// Defaults
fn default_search_concurrency() -> u32 {
    2
}
fn default_scrape_concurrency() -> u32 {
    2
}
fn default_max_search_results() -> u32 {
    5
}
fn default_max_scrape_chars() -> u32 {
    15000
}
fn default_context_window() -> u32 {
    8192
}
fn default_chunk_size() -> u32 {
    4000
}
fn default_temperature() -> f32 {
    0.7
}
fn default_top_p() -> f32 {
    0.9
}
fn default_scrape_timeout() -> u32 {
    30
}
fn default_vector_dims() -> u32 {
    384
}
fn default_sd_threads() -> u32 {
    0
}
fn default_false() -> bool {
    false
}
fn default_persona() -> String {
    "thinclaw".to_string()
}
fn default_memory_reservation() -> u32 {
    4
}

fn default_true() -> bool {
    true
}

fn default_spotlight_shortcut() -> String {
    "Command+Shift+K".to_string()
}

fn default_ptt_shortcut() -> String {
    "Command+Shift+V".to_string()
}

fn default_mcp_cache_ttl() -> u32 {
    300
}

fn default_mcp_tool_result_max_chars() -> u32 {
    5000
}

fn default_mcp_base_url() -> Option<String> {
    std::env::var("THINCLAW_MCP_URL")
        .or_else(|_| std::env::var("SCRAPPY_MCP_URL"))
        .ok()
        .filter(|s| !s.is_empty())
}

fn default_mcp_auth_token() -> Option<String> {
    std::env::var("THINCLAW_MCP_TOKEN")
        .or_else(|_| std::env::var("SCRAPPY_MCP_TOKEN"))
        .ok()
        .filter(|s| !s.is_empty())
}

fn normalize_user_config(mut config: UserConfig) -> UserConfig {
    if config.selected_persona == "scrappy" {
        config.selected_persona = default_persona();
    }
    config
}

pub struct ConfigManager {
    config: Mutex<UserConfig>,
    config_path: PathBuf,
    database: RwLock<Option<Arc<dyn thinclaw_core::db::Database>>>,
    write_lock: tokio::sync::Mutex<()>,
}

impl ConfigManager {
    pub fn new(app_handle: &AppHandle) -> Self {
        let config_path = app_handle
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("user_config.json");

        let config = if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                let loaded: UserConfig = serde_json::from_str(&content).unwrap_or_default();
                normalize_user_config(loaded)
            } else {
                UserConfig::default()
            }
        } else {
            let default = UserConfig::default();
            // Try to create the file
            if let Ok(json) = serde_json::to_string_pretty(&default) {
                let _ = std::fs::create_dir_all(config_path.parent().unwrap());
                let _ = std::fs::write(&config_path, json);
            }
            default
        };

        Self {
            config: Mutex::new(config),
            config_path,
            database: RwLock::new(None),
            write_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn get_config(&self) -> UserConfig {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Attach the canonical app-wide settings database and perform the one-time
    /// `user_config.json` merge. The database wins once a Workbench value has
    /// been written; the JSON file remains a recovery mirror for downgrades.
    pub async fn attach_database(
        &self,
        database: Arc<dyn thinclaw_core::db::Database>,
    ) -> Result<(), String> {
        let _write = self.write_lock.lock().await;
        let canonical = database
            .get_setting(SETTINGS_USER_ID, WORKBENCH_CONFIG_KEY)
            .await
            .map_err(|error| error.to_string())?;

        let config = if let Some(value) = canonical {
            normalize_user_config(
                serde_json::from_value(value)
                    .map_err(|error| format!("invalid canonical Workbench config: {error}"))?,
            )
        } else {
            let config = self.get_config();
            database
                .set_setting(
                    SETTINGS_USER_ID,
                    WORKBENCH_CONFIG_KEY,
                    &serde_json::to_value(&config).map_err(|error| error.to_string())?,
                )
                .await
                .map_err(|error| error.to_string())?;
            config
        };

        database
            .set_setting(
                SETTINGS_USER_ID,
                SETTINGS_SCHEMA_VERSION_KEY,
                &serde_json::json!(SETTINGS_SCHEMA_VERSION),
            )
            .await
            .map_err(|error| error.to_string())?;
        *self
            .database
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(database);
        *self
            .config
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = config.clone();
        if let Err(error) = self.write_recovery_file(&config).await {
            tracing::warn!(%error, "Failed to refresh legacy Workbench config recovery file");
        }
        Ok(())
    }

    pub async fn save_config(&self, new_config: &UserConfig) -> Result<(), String> {
        let _write = self.write_lock.lock().await;
        let normalized = normalize_user_config(new_config.clone());
        let database = self
            .database
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(database) = database {
            database
                .set_setting(
                    SETTINGS_USER_ID,
                    WORKBENCH_CONFIG_KEY,
                    &serde_json::to_value(&normalized).map_err(|error| error.to_string())?,
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        *self.config.lock().unwrap_or_else(|e| e.into_inner()) = normalized.clone();
        if let Err(error) = self.write_recovery_file(&normalized).await {
            tracing::warn!(%error, "Failed to update legacy Workbench config recovery file");
        }
        Ok(())
    }

    async fn write_recovery_file(&self, config: &UserConfig) -> Result<(), String> {
        let json = serde_json::to_string_pretty(config).map_err(|error| error.to_string())?;
        if let Some(parent) = self.config_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| error.to_string())?;
        }
        tokio::fs::write(&self.config_path, json)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn reload(&self) -> Result<(), String> {
        let _write = self.write_lock.lock().await;
        let database = self
            .database
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let config = if let Some(database) = database {
            database
                .get_setting(SETTINGS_USER_ID, WORKBENCH_CONFIG_KEY)
                .await
                .map_err(|error| error.to_string())?
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| error.to_string())?
        } else {
            tokio::fs::read_to_string(&self.config_path)
                .await
                .ok()
                .and_then(|content| serde_json::from_str(&content).ok())
        };
        if let Some(config) = config {
            *self
                .config
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = normalize_user_config(config);
        }
        Ok(())
    }

    fn canonical_database(&self) -> Result<Arc<dyn thinclaw_core::db::Database>, String> {
        self.database
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| "Canonical settings database is not initialized".to_string())
    }

    pub async fn agent_settings(&self) -> Result<serde_json::Value, String> {
        let database = self.canonical_database()?;
        let response = thinclaw_core::api::config::list_settings(&database, SETTINGS_USER_ID)
            .await
            .map_err(|error| error.to_string())?;
        let mut value = serde_json::to_value(response).map_err(|error| error.to_string())?;
        if let Some(settings) = value
            .get_mut("settings")
            .and_then(serde_json::Value::as_array_mut)
        {
            settings.retain(|setting| {
                setting
                    .get("key")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|key| !key.starts_with("desktop."))
            });
        }
        Ok(value)
    }

    pub async fn set_agent_setting(
        &self,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        if key.starts_with("desktop.") {
            return Err("desktop.* settings are reserved for the Workbench view".to_string());
        }
        let database = self.canonical_database()?;
        thinclaw_core::api::config::set_setting(&database, SETTINGS_USER_ID, key, value)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn patch_workbench(&self, patch: &serde_json::Value) -> Result<(), String> {
        let patch = patch
            .as_object()
            .ok_or_else(|| "Workbench settings patch must be an object".to_string())?;
        let mut merged =
            serde_json::to_value(self.get_config()).map_err(|error| error.to_string())?;
        let target = merged
            .as_object_mut()
            .ok_or_else(|| "Workbench settings must serialize as an object".to_string())?;
        for (key, value) in patch {
            if !target.contains_key(key) {
                return Err(format!("Unknown Workbench setting: {key}"));
            }
            target.insert(key.clone(), value.clone());
        }
        let config: UserConfig =
            serde_json::from_value(merged).map_err(|error| error.to_string())?;
        self.save_config(&config).await
    }
}

fn schema_for_default(value: &serde_json::Value) -> serde_json::Value {
    let type_name = match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    };
    let mut schema = serde_json::json!({ "type": type_name, "default": value });
    if let Some(object) = value.as_object() {
        schema["properties"] = serde_json::Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), schema_for_default(value)))
                .collect(),
        );
    }
    schema
}

pub fn unified_settings_schema() -> serde_json::Value {
    let mut workbench =
        serde_json::to_value(UserConfig::default()).unwrap_or_else(|_| serde_json::json!({}));
    // Secret values are managed by the dedicated keychain surface and must
    // never be reflected into a generic settings schema/default payload.
    if let Some(workbench) = workbench.as_object_mut() {
        workbench.remove("mcp_auth_token");
    }
    serde_json::json!({
        "version": SETTINGS_SCHEMA_VERSION,
        "type": "object",
        "views": {
            "workbench": {
                "title": "Direct AI Workbench",
                "description": "Direct chat, inference, RAG, media, and local engine preferences.",
                "storageKey": WORKBENCH_CONFIG_KEY,
                "schema": schema_for_default(&workbench),
            },
            "agent": {
                "title": "ThinClaw Agent Cockpit",
                "description": "Agent-runtime settings stored in the same canonical settings table.",
                "schema": { "type": "object", "additionalProperties": true },
            }
        }
    })
}

#[tauri::command]
#[specta::specta]
pub fn open_config_file(app: AppHandle) -> Result<(), String> {
    let config_path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("user_config.json");

    if !config_path.exists() {
        return Err("Config file does not exist yet".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(&config_path)
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    open::that(config_path.parent().unwrap_or(&config_path)).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_hf_token(app: AppHandle) -> Result<Option<String>, String> {
    if let Some(ironclaw) = app.try_state::<crate::thinclaw::runtime_bridge::ThinClawRuntimeState>()
    {
        if ironclaw.remote_proxy().await.is_some() {
            return Ok(None);
        }
    }

    // Read from the app-level SecretStore (NOT ThinClawConfig)
    if let Some(store) = app.try_state::<crate::secret_store::SecretStore>() {
        if let Some(token) = store.huggingface_token() {
            if !token.trim().is_empty() {
                println!("[config] get_hf_token: success (from SecretStore)");
                return Ok(Some(token));
            }
        }
    }

    println!("[config] get_hf_token: NoEntry (SecretStore)");
    Ok(None)
}

#[tauri::command]
#[specta::specta]
pub fn get_user_config(state: tauri::State<ConfigManager>) -> UserConfig {
    state.get_config()
}

#[tauri::command]
#[specta::specta]
pub async fn update_user_config(
    state: tauri::State<'_, ConfigManager>,
    config: UserConfigPatch,
) -> Result<(), String> {
    state.patch_workbench(&config.into_json()?).await
}

// =============================================================================
// Unit Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // A process-global lock that serialises tests which mutate environment variables.
    // Rust runs tests in parallel by default; std::env is process-global, so env
    // mutations must be done under a shared lock to avoid race conditions.
    static ENV_MUTEX: StdMutex<()> = StdMutex::new(());

    // -------------------------------------------------------------------------
    // Default values
    // -------------------------------------------------------------------------

    #[test]
    fn user_config_default_values_are_sane() {
        let cfg = UserConfig::default();
        assert_eq!(cfg.search_concurrency_limit, 2);
        assert_eq!(cfg.scrape_concurrency_limit, 2);
        assert_eq!(cfg.max_search_results, 5);
        assert_eq!(cfg.max_scrape_chars, 15_000);
        assert_eq!(cfg.default_context_window, 8192);
        assert_eq!(cfg.summarization_chunk_size, 4000);
        assert!((cfg.llm_temperature - 0.7).abs() < f32::EPSILON);
        assert!((cfg.llm_top_p - 0.9).abs() < f32::EPSILON);
        assert_eq!(cfg.scrape_timeout_secs, 30);
        assert_eq!(cfg.vector_dimensions, 384);
        assert_eq!(cfg.sd_threads, 0);
        assert!(cfg.knowledge_bits.is_empty());
        assert!(cfg.custom_personas.is_empty());
        assert!(!cfg.image_prompt_enhance_enabled);
        assert_eq!(cfg.selected_persona, "thinclaw");
        assert!(cfg.selected_chat_provider.is_none());
        assert_eq!(cfg.memory_reservation_gb, 4);
        assert!(cfg.enable_memory_reservation);
        assert!(!cfg.mlock);
        assert!(!cfg.quantize_kv);
        assert_eq!(cfg.spotlight_shortcut, "Command+Shift+K");
        assert!(cfg.disabled_providers.is_empty());
        assert!(!cfg.mcp_sandbox_enabled);
    }

    #[test]
    fn mcp_defaults_are_correct() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var("THINCLAW_MCP_URL");
            std::env::remove_var("THINCLAW_MCP_TOKEN");
            std::env::remove_var("SCRAPPY_MCP_URL");
            std::env::remove_var("SCRAPPY_MCP_TOKEN");
        }
        let cfg = UserConfig::default();

        assert_eq!(cfg.mcp_cache_ttl_secs, 300);
        assert_eq!(cfg.mcp_tool_result_max_chars, 5000);
        assert!(
            cfg.mcp_base_url.is_none(),
            "mcp_base_url should be None when env var is unset"
        );
        assert!(
            cfg.mcp_auth_token.is_none(),
            "mcp_auth_token should be None when env var is unset"
        );
    }

    #[test]
    fn mcp_defaults_read_from_thinclaw_env() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("THINCLAW_MCP_URL", "https://api.example.com");
            std::env::set_var("THINCLAW_MCP_TOKEN", "tok_abc123");
            std::env::remove_var("SCRAPPY_MCP_URL");
            std::env::remove_var("SCRAPPY_MCP_TOKEN");
        }
        let cfg = UserConfig::default();
        // Ensure cleanup even if assertion panics
        unsafe {
            std::env::remove_var("THINCLAW_MCP_URL");
            std::env::remove_var("THINCLAW_MCP_TOKEN");
        }

        assert_eq!(cfg.mcp_base_url.as_deref(), Some("https://api.example.com"));
        assert_eq!(cfg.mcp_auth_token.as_deref(), Some("tok_abc123"));
    }

    #[test]
    fn mcp_defaults_read_from_legacy_env() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var("THINCLAW_MCP_URL");
            std::env::remove_var("THINCLAW_MCP_TOKEN");
            std::env::set_var("SCRAPPY_MCP_URL", "https://api.example.com");
            std::env::set_var("SCRAPPY_MCP_TOKEN", "tok_abc123");
        }
        let cfg = UserConfig::default();
        // Ensure cleanup even if assertion panics
        unsafe {
            std::env::remove_var("SCRAPPY_MCP_URL");
            std::env::remove_var("SCRAPPY_MCP_TOKEN");
        }

        assert_eq!(cfg.mcp_base_url.as_deref(), Some("https://api.example.com"));
        assert_eq!(cfg.mcp_auth_token.as_deref(), Some("tok_abc123"));
    }

    #[test]
    fn legacy_scrappy_persona_is_normalized_for_new_writes() {
        let cfg = UserConfig {
            selected_persona: "scrappy".to_string(),
            ..UserConfig::default()
        };

        assert_eq!(normalize_user_config(cfg).selected_persona, "thinclaw");
    }

    // -------------------------------------------------------------------------
    // Serde round-trip
    // -------------------------------------------------------------------------

    #[test]
    fn user_config_serializes_and_deserializes_losslessly() {
        let original = UserConfig {
            selected_chat_provider: Some("anthropic".to_string()),
            mcp_base_url: Some("https://mcp.example.com".to_string()),
            mcp_auth_token: Some("secret".to_string()),
            mcp_cache_ttl_secs: 600,
            mcp_tool_result_max_chars: 10_000,
            mcp_sandbox_enabled: true,
            ..UserConfig::default()
        };

        let json = serde_json::to_string(&original).expect("serialization failed");
        let restored: UserConfig = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(
            restored.selected_chat_provider,
            original.selected_chat_provider
        );
        assert_eq!(restored.mcp_base_url, original.mcp_base_url);
        assert_eq!(restored.mcp_auth_token, original.mcp_auth_token);
        assert_eq!(restored.mcp_cache_ttl_secs, 600);
        assert_eq!(restored.mcp_tool_result_max_chars, 10_000);
        assert!(restored.mcp_sandbox_enabled);
    }

    #[test]
    fn partial_json_with_missing_fields_uses_serde_defaults() {
        // Simulate a config file that predates the new mcp_cache_ttl_secs field.
        let minimal_json = r#"{ "selected_persona": "thinclaw" }"#;
        let cfg: UserConfig = serde_json::from_str(minimal_json).expect("parse failed");

        // New fields should have their #[serde(default)] values
        assert_eq!(cfg.mcp_cache_ttl_secs, 300);
        assert_eq!(cfg.mcp_tool_result_max_chars, 5000);
        assert!(!cfg.mcp_sandbox_enabled);
    }

    // -------------------------------------------------------------------------
    // ConfigManager in-memory API (no filesystem needed)
    // -------------------------------------------------------------------------

    fn make_manager_from_config(cfg: UserConfig) -> ConfigManager {
        ConfigManager {
            config: Mutex::new(cfg),
            config_path: std::path::PathBuf::from("/tmp/thinclaw_desktop_test_config.json"),
            database: RwLock::new(None),
            write_lock: tokio::sync::Mutex::new(()),
        }
    }

    async fn canonical_database(temp: &tempfile::TempDir) -> Arc<dyn thinclaw_core::db::Database> {
        use thinclaw_core::db::Database as _;
        let path = temp.path().join("settings.db");
        let backend = thinclaw_db::libsql::LibSqlBackend::new_local(&path)
            .await
            .expect("open canonical settings database");
        backend.run_migrations().await.expect("run migrations");
        Arc::new(backend)
    }

    #[test]
    fn config_manager_get_returns_current_config() {
        let cfg = UserConfig {
            selected_persona: "custom".to_string(),
            ..UserConfig::default()
        };
        let mgr = make_manager_from_config(cfg);
        assert_eq!(mgr.get_config().selected_persona, "custom");
    }

    #[tokio::test]
    async fn config_manager_save_updates_in_memory_immediately() {
        let mgr = make_manager_from_config(UserConfig::default());

        let mut updated = mgr.get_config();
        updated.max_search_results = 99;
        updated.mcp_cache_ttl_secs = 120;
        mgr.save_config(&updated).await.expect("save config");

        // The in-memory state must be updated synchronously (disk write is async)
        let read_back = mgr.get_config();
        assert_eq!(read_back.max_search_results, 99);
        assert_eq!(read_back.mcp_cache_ttl_secs, 120);
    }

    #[tokio::test]
    async fn canonical_database_migrates_file_config_and_wins_on_restart() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let database = canonical_database(&temp).await;
        let original = UserConfig {
            max_search_results: 17,
            selected_persona: "migrated".to_string(),
            ..UserConfig::default()
        };
        let manager = ConfigManager {
            config: Mutex::new(original),
            config_path: temp.path().join("user_config.json"),
            database: RwLock::new(None),
            write_lock: tokio::sync::Mutex::new(()),
        };
        manager
            .attach_database(Arc::clone(&database))
            .await
            .expect("attach canonical database");

        let persisted = database
            .get_setting(SETTINGS_USER_ID, WORKBENCH_CONFIG_KEY)
            .await
            .expect("read canonical config")
            .expect("canonical config exists");
        assert_eq!(persisted["max_search_results"], 17);
        assert_eq!(persisted["selected_persona"], "migrated");

        let restarted = ConfigManager {
            config: Mutex::new(UserConfig::default()),
            config_path: temp.path().join("restarted-user_config.json"),
            database: RwLock::new(None),
            write_lock: tokio::sync::Mutex::new(()),
        };
        restarted
            .attach_database(Arc::clone(&database))
            .await
            .expect("reattach canonical database");
        assert_eq!(restarted.get_config().max_search_results, 17);
        assert_eq!(restarted.get_config().selected_persona, "migrated");

        let mut updated = restarted.get_config();
        updated.max_search_results = 23;
        restarted.save_config(&updated).await.expect("save update");
        let persisted = database
            .get_setting(SETTINGS_USER_ID, WORKBENCH_CONFIG_KEY)
            .await
            .expect("read updated config")
            .expect("updated config exists");
        assert_eq!(persisted["max_search_results"], 23);
    }

    #[tokio::test]
    async fn agent_and_workbench_views_share_storage_without_leaking_reserved_rows() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let database = canonical_database(&temp).await;
        let manager = ConfigManager {
            config: Mutex::new(UserConfig::default()),
            config_path: temp.path().join("user_config.json"),
            database: RwLock::new(None),
            write_lock: tokio::sync::Mutex::new(()),
        };
        manager
            .attach_database(database)
            .await
            .expect("attach canonical database");
        manager
            .set_agent_setting("llm.backend", &serde_json::json!("anthropic"))
            .await
            .expect("set agent setting");

        let response = manager.agent_settings().await.expect("list agent settings");
        let rows = response["settings"].as_array().expect("settings rows");
        assert!(rows.iter().any(|row| row["key"] == "llm.backend"));
        assert!(!rows.iter().any(|row| {
            row["key"]
                .as_str()
                .is_some_and(|key| key.starts_with("desktop."))
        }));
        assert!(manager
            .set_agent_setting("desktop.workbench", &serde_json::json!({}))
            .await
            .is_err());
    }

    #[test]
    fn unified_schema_is_derived_from_every_workbench_field() {
        let schema = unified_settings_schema();
        let properties = schema["views"]["workbench"]["schema"]["properties"]
            .as_object()
            .expect("workbench properties");
        let defaults = serde_json::to_value(UserConfig::default())
            .expect("serialize defaults")
            .as_object()
            .expect("default fields")
            .clone();
        assert_eq!(properties.len(), defaults.len() - 1);
        assert!(!properties.contains_key("mcp_auth_token"));
        assert!(defaults
            .keys()
            .filter(|key| key.as_str() != "mcp_auth_token")
            .all(|key| properties.contains_key(key)));
    }

    // -------------------------------------------------------------------------
    // Presence-aware command patch semantics
    // -------------------------------------------------------------------------

    #[test]
    fn user_config_patch_serializes_only_fields_the_caller_supplied() {
        let patch: UserConfigPatch = serde_json::from_value(serde_json::json!({
            "max_search_results": 15,
            "mcp_base_url": null
        }))
        .expect("deserialize patch");

        assert_eq!(
            patch.into_json().expect("serialize patch"),
            serde_json::json!({
                "max_search_results": 15,
                "mcp_base_url": null
            })
        );
    }

    #[test]
    fn user_config_patch_rejects_null_for_non_nullable_fields() {
        let error = serde_json::from_value::<UserConfigPatch>(serde_json::json!({
            "max_search_results": null
        }))
        .expect_err("non-nullable field must reject null");

        assert!(error.to_string().contains("invalid type"));
    }

    #[test]
    fn user_config_patch_rejects_unknown_fields() {
        let error = serde_json::from_value::<UserConfigPatch>(serde_json::json!({
            "raw": "legacy payload",
            "baseHash": "stale hash"
        }))
        .expect_err("legacy wrapper must not be silently accepted");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn json_merge_last_write_wins_on_basic_field() {
        let base: UserConfig = serde_json::from_str(r#"{ "max_search_results": 5 }"#).unwrap();
        let incoming: UserConfig = serde_json::from_str(r#"{ "max_search_results": 20 }"#).unwrap();

        let mut base_val = serde_json::to_value(&base).unwrap();
        let inc_val = serde_json::to_value(&incoming).unwrap();

        if let (Some(b), Some(i)) = (base_val.as_object_mut(), inc_val.as_object()) {
            for (k, v) in i {
                b.insert(k.clone(), v.clone());
            }
        }

        let merged: UserConfig = serde_json::from_value(base_val).unwrap();
        assert_eq!(merged.max_search_results, 20);
    }

    #[test]
    fn json_merge_preserves_unrelated_fields_from_base() {
        // Build the base: a config that has a specific mcp_base_url set.
        let base = UserConfig {
            mcp_base_url: Some("https://keep-me.com".to_string()),
            max_search_results: 7,
            ..Default::default()
        };

        // The "incoming" patch only mentions max_search_results.
        // We build it as a raw JSON object so serde does NOT emit a null for
        // mcp_base_url (which would incorrectly overwrite the base value).
        // This matches the presence-aware UserConfigPatch representation used
        // by update_user_config.
        let inc_val: serde_json::Value = serde_json::json!({
            "max_search_results": 15
        });

        let mut base_val = serde_json::to_value(&base).unwrap();

        if let (Some(b), Some(i)) = (base_val.as_object_mut(), inc_val.as_object()) {
            for (k, v) in i {
                b.insert(k.clone(), v.clone());
            }
        }

        let merged: UserConfig = serde_json::from_value(base_val).unwrap();
        assert_eq!(merged.max_search_results, 15);
        // The unrelated field from base must survive the partial merge
        assert_eq!(
            merged.mcp_base_url.as_deref(),
            Some("https://keep-me.com"),
            "mcp_base_url should survive a merge that didn't include it"
        );
    }

    #[test]
    fn json_merge_null_incoming_field_overwrites_base() {
        // Documents the expected behaviour: if a field is explicitly set to null
        // in the incoming patch (e.g. user cleared the URL field), it SHOULD
        // overwrite the base value. This is the "last write wins" semantic.
        let base = UserConfig {
            mcp_base_url: Some("https://old-url.com".to_string()),
            ..Default::default()
        };

        // Explicitly null overwrites
        let inc_val = serde_json::json!({ "mcp_base_url": null });
        let mut base_val = serde_json::to_value(&base).unwrap();

        if let (Some(b), Some(i)) = (base_val.as_object_mut(), inc_val.as_object()) {
            for (k, v) in i {
                b.insert(k.clone(), v.clone());
            }
        }

        let merged: UserConfig = serde_json::from_value(base_val).unwrap();
        assert!(
            merged.mcp_base_url.is_none(),
            "explicit null in patch should clear the base field"
        );
    }
}
