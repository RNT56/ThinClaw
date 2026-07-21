use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

const MAX_USER_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_KNOWLEDGE_BITS: usize = 256;
const MAX_CUSTOM_PERSONAS: usize = 64;
const MAX_PERSONALIZATION_BYTES: usize = 256 * 1024;

fn bounded_config_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= max_bytes
        && !value.contains('\0')
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn valid_mcp_auth_token(token: &str) -> bool {
    token.trim() == token && token.len() <= 16 * 1024 && !token.chars().any(char::is_control)
}

fn validate_user_config(config: &UserConfig) -> Result<(), String> {
    if !(1..=8).contains(&config.search_concurrency_limit)
        || !(1..=8).contains(&config.scrape_concurrency_limit)
        || !(1..=20).contains(&config.max_search_results)
        || !(1_000..=200_000).contains(&config.max_scrape_chars)
        || !(1..=120).contains(&config.scrape_timeout_secs)
        || !(1_024..=2_000_000).contains(&config.default_context_window)
        || !(1_000..=32_000).contains(&config.summarization_chunk_size)
        || !config.llm_temperature.is_finite()
        || !(0.0..=2.0).contains(&config.llm_temperature)
        || !config.llm_top_p.is_finite()
        || !(0.0..=1.0).contains(&config.llm_top_p)
        || config.vector_dimensions == 0
        || config.vector_dimensions
            > u32::try_from(crate::inference::embedding::MAX_EMBEDDING_DIMENSIONS)
                .unwrap_or(u32::MAX)
        || config.sd_threads > 128
        || config.memory_reservation_gb > 1_024
        || !(1..=86_400).contains(&config.mcp_cache_ttl_secs)
        || !(1_000..=1_000_000).contains(&config.mcp_tool_result_max_chars)
        || config
            .selected_model_context_size
            .is_some_and(|value| !(1_024..=2_000_000).contains(&value))
    {
        return Err("User configuration contains an out-of-range numeric setting".to_string());
    }

    if !bounded_config_text(&config.selected_persona, 256, false)
        || !bounded_config_text(&config.spotlight_shortcut, 128, false)
        || !bounded_config_text(&config.ptt_shortcut, 128, false)
        || config.disabled_providers.len() > 128
        || config
            .disabled_providers
            .iter()
            .any(|provider| !bounded_config_text(provider, 128, false))
    {
        return Err("User configuration contains invalid identifiers or shortcuts".to_string());
    }

    let mut personalization_bytes = 0usize;
    let mut knowledge_ids = std::collections::HashSet::new();
    if config.knowledge_bits.len() > MAX_KNOWLEDGE_BITS {
        return Err("User configuration contains too many knowledge entries".to_string());
    }
    for bit in &config.knowledge_bits {
        if !bounded_config_text(&bit.id, 256, false)
            || !bounded_config_text(&bit.label, 1_024, false)
            || !bounded_config_text(&bit.content, 64 * 1024, true)
            || !knowledge_ids.insert(&bit.id)
        {
            return Err("User configuration contains an invalid knowledge entry".to_string());
        }
        personalization_bytes = personalization_bytes
            .saturating_add(bit.label.len())
            .saturating_add(bit.content.len());
    }
    let mut persona_ids = std::collections::HashSet::new();
    if config.custom_personas.len() > MAX_CUSTOM_PERSONAS {
        return Err("User configuration contains too many custom personas".to_string());
    }
    for persona in &config.custom_personas {
        if !bounded_config_text(&persona.id, 256, false)
            || !bounded_config_text(&persona.name, 1_024, false)
            || !bounded_config_text(&persona.description, 16 * 1024, true)
            || !bounded_config_text(&persona.instructions, 128 * 1024, false)
            || !persona_ids.insert(&persona.id)
        {
            return Err("User configuration contains an invalid custom persona".to_string());
        }
        personalization_bytes = personalization_bytes
            .saturating_add(persona.name.len())
            .saturating_add(persona.description.len())
            .saturating_add(persona.instructions.len());
    }
    if personalization_bytes > MAX_PERSONALIZATION_BYTES {
        return Err("User personalization exceeds the aggregate size limit".to_string());
    }

    for backend in [
        config.selected_chat_provider.as_deref(),
        config.chat_backend.as_deref(),
        config.embedding_backend.as_deref(),
        config.tts_backend.as_deref(),
        config.stt_backend.as_deref(),
        config.diffusion_backend.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !bounded_config_text(backend, 128, false) {
            return Err("User configuration contains an invalid backend identifier".to_string());
        }
    }
    if config.inference_models.as_ref().is_some_and(|models| {
        models.len() > 32
            || models.iter().any(|(modality, model)| {
                !bounded_config_text(modality, 64, false) || !bounded_config_text(model, 512, false)
            })
    }) {
        return Err("User configuration contains invalid inference models".to_string());
    }

    if config
        .mcp_auth_token
        .as_deref()
        .is_some_and(|token| !valid_mcp_auth_token(token))
    {
        return Err("MCP authentication token is invalid".to_string());
    }
    if let Some(base_url) = config.mcp_base_url.as_deref() {
        thinclaw_desktop_tools::McpClient::new(thinclaw_desktop_tools::McpConfig {
            base_url: base_url.to_string(),
            auth_token: config.mcp_auth_token.clone().unwrap_or_default(),
            timeout_ms: 30_000,
        })
        .map_err(|_| "MCP endpoint configuration is invalid".to_string())?;
    } else if config.mcp_sandbox_enabled {
        return Err("MCP sandbox mode requires an MCP endpoint".to_string());
    }
    Ok(())
}

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

#[derive(Clone, Serialize, Deserialize, specta::Type)]
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

/// Presence-aware update payload for [`UserConfig`]. Missing fields retain
/// their latest backend value; `PatchField<Option<T>>` also preserves the
/// distinction between omission and an explicit `null` clear operation.
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
    fn into_json(self) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        serde_json::to_value(self)
            .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))
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

impl std::fmt::Debug for UserConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserConfig")
            .field("search_concurrency_limit", &self.search_concurrency_limit)
            .field("scrape_concurrency_limit", &self.scrape_concurrency_limit)
            .field("max_search_results", &self.max_search_results)
            .field("max_scrape_chars", &self.max_scrape_chars)
            .field("default_context_window", &self.default_context_window)
            .field("knowledge_bit_count", &self.knowledge_bits.len())
            .field("custom_persona_count", &self.custom_personas.len())
            .field("selected_persona", &self.selected_persona)
            .field("selected_chat_provider", &self.selected_chat_provider)
            .field("mcp_base_url_configured", &self.mcp_base_url.is_some())
            .field(
                "mcp_auth_token",
                &crate::debug_redaction::RedactedOption(&self.mcp_auth_token),
            )
            .field("mcp_sandbox_enabled", &self.mcp_sandbox_enabled)
            .field("mcp_cache_ttl_secs", &self.mcp_cache_ttl_secs)
            .field("mcp_tool_result_max_chars", &self.mcp_tool_result_max_chars)
            .field("chat_backend", &self.chat_backend)
            .field("embedding_backend", &self.embedding_backend)
            .field("tts_backend", &self.tts_backend)
            .field("stt_backend", &self.stt_backend)
            .field("diffusion_backend", &self.diffusion_backend)
            .finish_non_exhaustive()
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
        .filter(|value| {
            !value.is_empty()
                && thinclaw_desktop_tools::McpClient::new(thinclaw_desktop_tools::McpConfig {
                    base_url: value.clone(),
                    auth_token: String::new(),
                    timeout_ms: 30_000,
                })
                .is_ok()
        })
}

fn default_mcp_auth_token() -> Option<String> {
    std::env::var("THINCLAW_MCP_TOKEN")
        .or_else(|_| std::env::var("SCRAPPY_MCP_TOKEN"))
        .ok()
        .filter(|value| !value.is_empty() && valid_mcp_auth_token(value))
}

pub(crate) const MCP_AUTH_TOKEN_SECRET_KEY: &str = "desktop_mcp_auth_token";

fn config_json_for_persistence(config: &UserConfig) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(config)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("mcp_auth_token");
    }
    serde_json::to_string_pretty(&value)
}

pub(crate) fn write_config_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if contents.len() > MAX_USER_CONFIG_BYTES as usize {
        return Err("user configuration exceeds the size limit".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "user config path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create user config directory: {error}"))?;
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("failed to inspect user config directory: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("user config directory is not a real directory".to_string());
    }
    thinclaw_platform::write_private_file_atomic(path, contents.as_bytes(), true)
        .map_err(|error| format!("failed to atomically publish user config: {error}"))
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
    mutation_lock: Mutex<()>,
}

impl ConfigManager {
    pub fn new(app_handle: &AppHandle) -> Self {
        let config_path = app_handle
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("user_config.json");

        let mut legacy_mcp_token = None;
        let mut legacy_scrubbed_json = None;
        let metadata = std::fs::symlink_metadata(&config_path);
        let should_create_default = metadata
            .as_ref()
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
        let mut config = match metadata {
            Ok(_) => match thinclaw_platform::read_regular_file_bounded_single_link(
                &config_path,
                MAX_USER_CONFIG_BYTES,
            ) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(content) => {
                        if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&content) {
                            legacy_mcp_token = value
                                .get("mcp_auth_token")
                                .and_then(serde_json::Value::as_str)
                                .filter(|token| !token.is_empty() && valid_mcp_auth_token(token))
                                .map(str::to_owned);
                            if legacy_mcp_token.is_some() {
                                if let Some(object) = value.as_object_mut() {
                                    object.remove("mcp_auth_token");
                                }
                                legacy_scrubbed_json = serde_json::to_string_pretty(&value).ok();
                            }
                        }
                        match serde_json::from_str::<UserConfig>(&content)
                            .map(normalize_user_config)
                        {
                            Ok(loaded) if validate_user_config(&loaded).is_ok() => loaded,
                            Ok(_) => {
                                tracing::warn!(
                                    "User configuration failed validation; using defaults"
                                );
                                UserConfig::default()
                            }
                            Err(error) => {
                                tracing::warn!(
                                    "User configuration could not be decoded; using defaults: {error}"
                                );
                                UserConfig::default()
                            }
                        }
                    }
                    Err(_) => {
                        tracing::warn!("User configuration is not valid UTF-8; using defaults");
                        UserConfig::default()
                    }
                },
                Err(error) => {
                    tracing::warn!("User configuration could not be read safely: {error}");
                    UserConfig::default()
                }
            },
            Err(error) => {
                if !should_create_default {
                    tracing::warn!("User configuration could not be inspected: {error}");
                }
                UserConfig::default()
            }
        };

        if should_create_default {
            match config_json_for_persistence(&config) {
                Ok(json) => {
                    if let Err(error) = write_config_file(&config_path, &json) {
                        tracing::warn!("Failed to persist default user configuration: {error}");
                    }
                }
                Err(error) => {
                    tracing::warn!("Failed to encode default user configuration: {error}")
                }
            }
        }

        let mut migrated_legacy_token = false;
        if let Some(token) = legacy_mcp_token.as_deref() {
            match crate::thinclaw::config::keychain::set_key(MCP_AUTH_TOKEN_SECRET_KEY, Some(token))
            {
                Ok(()) => migrated_legacy_token = true,
                Err(error) => tracing::warn!(
                    "Failed to migrate legacy MCP credential to secure storage: {error}"
                ),
            }
            if migrated_legacy_token {
                if let Some(json) = legacy_scrubbed_json.as_deref() {
                    if let Err(error) = write_config_file(&config_path, json) {
                        tracing::warn!(
                            "MCP credential was secured but legacy config could not be scrubbed: {error}"
                        );
                    }
                } else {
                    tracing::warn!(
                        "MCP credential was secured but the legacy config could not be scrubbed safely"
                    );
                }
            }
        }
        config.mcp_auth_token =
            crate::thinclaw::config::keychain::get_key(MCP_AUTH_TOKEN_SECRET_KEY)
                .or_else(|| legacy_mcp_token.filter(|_| !migrated_legacy_token))
                .or_else(default_mcp_auth_token);

        Self {
            config: Mutex::new(config),
            config_path,
            mutation_lock: Mutex::new(()),
        }
    }

    pub fn get_config(&self) -> UserConfig {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn save_config(&self, new_config: &UserConfig) -> Result<(), String> {
        let normalized = normalize_user_config(new_config.clone());
        validate_user_config(&normalized)?;
        let json = config_json_for_persistence(&normalized).map_err(|error| error.to_string())?;
        let mut config = self
            .config
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        write_config_file(&self.config_path, &json)?;
        *config = normalized;
        Ok(())
    }

    pub fn reload(&self) {
        if let Ok(bytes) = thinclaw_platform::read_regular_file_bounded_single_link(
            &self.config_path,
            MAX_USER_CONFIG_BYTES,
        ) {
            if let Ok(new_config) = serde_json::from_slice(&bytes) {
                let mut normalized = normalize_user_config(new_config);
                normalized.mcp_auth_token =
                    crate::thinclaw::config::keychain::get_key(MCP_AUTH_TOKEN_SECRET_KEY)
                        .or_else(default_mcp_auth_token);
                if validate_user_config(&normalized).is_ok() {
                    *self.config.lock().unwrap_or_else(|e| e.into_inner()) = normalized;
                }
            }
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn open_config_file(app: AppHandle) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let config_path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("user_config.json");

    thinclaw_platform::read_regular_file_bounded_single_link(&config_path, MAX_USER_CONFIG_BYTES)
        .map_err(|error| format!("Config file cannot be opened safely: {error}"))?;

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&config_path)
            .spawn()
            .map_err(|error| format!("Failed to open config file: {error}"))?;
    }
    #[cfg(not(target_os = "macos"))]
    open::that(&config_path).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_hf_token(
    app: AppHandle,
) -> Result<Option<String>, crate::thinclaw::bridge::BridgeError> {
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
pub fn get_user_config(
    state: tauri::State<ConfigManager>,
    secret_store: tauri::State<crate::secret_store::SecretStore>,
) -> UserConfig {
    let mut config = state.get_config();
    config.mcp_auth_token = secret_store
        .get(MCP_AUTH_TOKEN_SECRET_KEY)
        .or_else(default_mcp_auth_token);
    config
}

#[tauri::command]
#[specta::specta]
pub fn update_user_config(
    state: tauri::State<ConfigManager>,
    secret_store: tauri::State<crate::secret_store::SecretStore>,
    config: UserConfigPatch,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let _mutation = state
        .mutation_lock
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let patch = config.into_json()?;
    let patch = patch
        .as_object()
        .ok_or_else(|| "User configuration patch must be an object".to_string())?;
    let token_change = patch.get("mcp_auth_token").map(|value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_owned)
    });

    let mut merged = serde_json::to_value(state.get_config())
        .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
    let target = merged
        .as_object_mut()
        .ok_or_else(|| "User configuration must serialize as an object".to_string())?;
    for (key, value) in patch {
        if !target.contains_key(key) {
            return Err(format!("Unknown user configuration field: {key}").into());
        }
        target.insert(key.clone(), value.clone());
    }

    let mut merged: UserConfig = serde_json::from_value(merged)
        .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
    if let Some(value) = &token_change {
        merged.mcp_auth_token = value.clone().or_else(default_mcp_auth_token);
    }
    validate_user_config(&normalize_user_config(merged.clone()))?;

    let previous_mcp_token = secret_store.get(MCP_AUTH_TOKEN_SECRET_KEY);
    if let Some(value) = &token_change {
        secret_store.set(MCP_AUTH_TOKEN_SECRET_KEY, value.as_deref())?;
    }
    if let Err(save_error) = state.save_config(&merged) {
        if token_change.is_none() {
            return Err(save_error.into());
        }
        return match secret_store.set(MCP_AUTH_TOKEN_SECRET_KEY, previous_mcp_token.as_deref()) {
            Ok(()) => Err(save_error.into()),
            Err(rollback_error) => Err(format!(
                "{save_error}; additionally failed to restore the previous MCP credential: {rollback_error}"
            )
            .into()),
        };
    }
    Ok(())
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
    fn invalid_mcp_environment_defaults_are_ignored() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("THINCLAW_MCP_URL", "http://public.example.com");
            std::env::set_var("THINCLAW_MCP_TOKEN", "bad\ntoken");
            std::env::remove_var("SCRAPPY_MCP_URL");
            std::env::remove_var("SCRAPPY_MCP_TOKEN");
        }
        let config = UserConfig::default();
        unsafe {
            std::env::remove_var("THINCLAW_MCP_URL");
            std::env::remove_var("THINCLAW_MCP_TOKEN");
        }

        assert!(config.mcp_base_url.is_none());
        assert!(config.mcp_auth_token.is_none());
        assert!(validate_user_config(&config).is_ok());
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
    fn persisted_user_config_omits_mcp_token_and_debug_redacts_private_content() {
        let config = UserConfig {
            mcp_auth_token: Some("mcp-live-secret".into()),
            knowledge_bits: vec![KnowledgeBit {
                id: "private".into(),
                label: "Private".into(),
                content: "private-knowledge-content".into(),
                enabled: true,
            }],
            ..UserConfig::default()
        };

        let persisted = config_json_for_persistence(&config).unwrap();
        assert!(!persisted.contains("mcp-live-secret"));
        assert!(!persisted.contains("mcp_auth_token"));

        let debug = format!("{config:?}");
        assert!(!debug.contains("mcp-live-secret"));
        assert!(!debug.contains("private-knowledge-content"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn user_config_validation_rejects_unsafe_endpoints_and_oversized_personalization() {
        let remote_http = UserConfig {
            mcp_base_url: Some("http://public.example.com".into()),
            ..UserConfig::default()
        };
        assert!(validate_user_config(&remote_http).is_err());

        let oversized = UserConfig {
            knowledge_bits: (0..5)
                .map(|index| KnowledgeBit {
                    id: format!("knowledge-{index}"),
                    label: "label".into(),
                    content: "x".repeat(64 * 1024),
                    enabled: true,
                })
                .collect(),
            ..UserConfig::default()
        };
        assert!(validate_user_config(&oversized).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn config_writer_rejects_a_symlink_target() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let target = directory.path().join("user_config.json");
        std::os::unix::fs::symlink(outside.path(), &target).unwrap();
        assert!(write_config_file(&target, "{}").is_err());
        assert_eq!(std::fs::read(outside.path()).unwrap(), Vec::<u8>::new());
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
            mutation_lock: Mutex::new(()),
        }
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

    #[test]
    fn config_manager_save_updates_in_memory_after_durable_write() {
        let directory = tempfile::tempdir().unwrap();
        let mgr = ConfigManager {
            config: Mutex::new(UserConfig::default()),
            config_path: directory.path().join("user_config.json"),
            mutation_lock: Mutex::new(()),
        };

        let mut updated = mgr.get_config();
        updated.max_search_results = 19;
        updated.mcp_cache_ttl_secs = 120;
        updated.mcp_auth_token = Some("not-persisted".into());
        mgr.save_config(&updated).unwrap();

        let read_back = mgr.get_config();
        assert_eq!(read_back.max_search_results, 19);
        assert_eq!(read_back.mcp_cache_ttl_secs, 120);
        assert_eq!(read_back.mcp_auth_token.as_deref(), Some("not-persisted"));

        let persisted = std::fs::read_to_string(&mgr.config_path).unwrap();
        assert!(!persisted.contains("not-persisted"));
        assert!(!persisted.contains("mcp_auth_token"));
    }

    // -------------------------------------------------------------------------
    // JSON-level merge semantics (update_user_config logic)
    // -------------------------------------------------------------------------

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
        // This matches how update_user_config actually receives data from the
        // frontend — partial objects, not full UserConfig serialisations.
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
