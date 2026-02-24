use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

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

    #[serde(default)]
    pub disabled_providers: Vec<String>,

    // --- MCP Integration ---
    /// MCP server base URL (e.g. "https://api.scrappy.dev")
    /// Falls back to SCRAPPY_MCP_URL env var if not set in config.
    #[serde(default = "default_mcp_base_url")]
    pub mcp_base_url: Option<String>,

    /// MCP JWT auth token. Falls back to SCRAPPY_MCP_TOKEN env var.
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
            disabled_providers: vec![],
            mcp_base_url: default_mcp_base_url(),
            mcp_auth_token: default_mcp_auth_token(),
            mcp_sandbox_enabled: false,
            mcp_cache_ttl_secs: default_mcp_cache_ttl(),
            mcp_tool_result_max_chars: default_mcp_tool_result_max_chars(),
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
    "scrappy".to_string()
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

fn default_mcp_cache_ttl() -> u32 {
    300
}

fn default_mcp_tool_result_max_chars() -> u32 {
    5000
}

fn default_mcp_base_url() -> Option<String> {
    std::env::var("SCRAPPY_MCP_URL")
        .ok()
        .filter(|s| !s.is_empty())
}

fn default_mcp_auth_token() -> Option<String> {
    std::env::var("SCRAPPY_MCP_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

pub struct ConfigManager {
    config: Mutex<UserConfig>,
    config_path: PathBuf,
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
                loaded
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
        }
    }

    pub fn get_config(&self) -> UserConfig {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn save_config(&self, new_config: &UserConfig) {
        *self.config.lock().unwrap_or_else(|e| e.into_inner()) = new_config.clone();
        // Spawn an async write so we never block the Tokio thread-pool under
        // lock contention (the in-memory cache above is already updated).
        if let Ok(json) = serde_json::to_string_pretty(new_config) {
            let path = self.config_path.clone();
            tauri::async_runtime::spawn(async move {
                let _ = tokio::fs::write(&path, json).await;
            });
        }
    }

    pub fn reload(&self) {
        if let Ok(content) = std::fs::read_to_string(&self.config_path) {
            if let Ok(new_config) = serde_json::from_str(&content) {
                *self.config.lock().unwrap_or_else(|e| e.into_inner()) = new_config;
            }
        }
    }
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
        let _ = std::process::Command::new("open").arg(&config_path).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    open::that(&config_path).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_hf_token(app: AppHandle) -> Result<Option<String>, String> {
    // Read from the app-level SecretStore (NOT OpenClawConfig)
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
pub fn update_user_config(
    state: tauri::State<ConfigManager>,
    config: UserConfig,
) -> Result<(), String> {
    // JSON-level merge: read current config, overlay incoming fields, save.
    // This prevents a stale frontend copy from overwriting concurrent backend
    // changes (e.g. local inference config written by the sidecar manager).
    let current = state.get_config();
    let mut base = serde_json::to_value(&current).map_err(|e| e.to_string())?;
    let incoming = serde_json::to_value(&config).map_err(|e| e.to_string())?;

    if let (Some(base_obj), Some(inc_obj)) = (base.as_object_mut(), incoming.as_object()) {
        for (key, value) in inc_obj {
            base_obj.insert(key.clone(), value.clone());
        }
    }

    let merged: UserConfig = serde_json::from_value(base).map_err(|e| e.to_string())?;
    state.save_config(&merged);
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
        assert_eq!(cfg.selected_persona, "scrappy");
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
    fn mcp_defaults_read_from_env() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
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
        let minimal_json = r#"{ "selected_persona": "scrappy" }"#;
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
            config_path: std::path::PathBuf::from("/tmp/scrappy_test_config.json"),
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

    #[tokio::test]
    async fn config_manager_save_updates_in_memory_immediately() {
        let mgr = make_manager_from_config(UserConfig::default());

        let mut updated = mgr.get_config();
        updated.max_search_results = 99;
        updated.mcp_cache_ttl_secs = 120;
        mgr.save_config(&updated);

        // The in-memory state must be updated synchronously (disk write is async)
        let read_back = mgr.get_config();
        assert_eq!(read_back.max_search_results, 99);
        assert_eq!(read_back.mcp_cache_ttl_secs, 120);
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
        let mut base = UserConfig::default();
        base.mcp_base_url = Some("https://keep-me.com".to_string());
        base.max_search_results = 7;

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
        let mut base = UserConfig::default();
        base.mcp_base_url = Some("https://old-url.com".to_string());

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
