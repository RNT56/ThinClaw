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

    #[serde(default = "default_false")]
    pub image_prompt_enhance_enabled: bool,

    #[serde(default = "default_persona")]
    pub selected_persona: String,
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
            image_prompt_enhance_enabled: false,
            selected_persona: default_persona(),
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
        self.config.lock().unwrap().clone()
    }

    pub fn save_config(&self, new_config: &UserConfig) {
        *self.config.lock().unwrap() = new_config.clone();
        if let Ok(json) = serde_json::to_string_pretty(new_config) {
            let _ = std::fs::write(&self.config_path, json);
        }
    }

    pub fn reload(&self) {
        if let Ok(content) = std::fs::read_to_string(&self.config_path) {
            if let Ok(new_config) = serde_json::from_str(&content) {
                *self.config.lock().unwrap() = new_config;
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
pub fn get_hf_token(app: AppHandle) -> Result<Option<String>, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let config = crate::clawdbot::ClawdbotConfig::new(app_data_dir);

    if let Some(token) = config.huggingface_token {
        if !token.trim().is_empty() {
            println!("[config] get_hf_token: success (from clawdbot config)");
            return Ok(Some(token));
        }
    }

    println!("[config] get_hf_token: NoEntry (clawdbot config)");
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
    state.save_config(&config);
    Ok(())
}
