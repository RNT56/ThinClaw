//! Plugin interfaces: auth, memory, and provider plugins.
//!
//! Defines the trait interfaces that WASM or native plugins can implement
//! to extend ThinClaw's auth, memory, and LLM provider capabilities.

use serde::{Deserialize, Serialize};

// ─── Auth Plugin ───

/// Trait interface for auth plugins.
pub trait AuthPlugin: Send + Sync {
    /// Plugin name.
    fn name(&self) -> &str;
    /// Authenticate a request. Returns a token or error.
    fn authenticate(&self, credentials: &AuthCredentials) -> Result<AuthToken, AuthPluginError>;
    /// Refresh an expired token.
    fn refresh(&self, token: &AuthToken) -> Result<AuthToken, AuthPluginError>;
    /// Validate a token.
    fn validate(&self, token: &AuthToken) -> bool;
}

/// Credentials provided to an auth plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCredentials {
    pub provider: String,
    pub username: Option<String>,
    pub api_key: Option<String>,
    pub extra: std::collections::HashMap<String, String>,
}

/// Token returned by an auth plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthToken {
    pub token: String,
    pub token_type: String,
    pub expires_at: Option<i64>,
    pub scopes: Vec<String>,
}

/// Auth plugin error.
#[derive(Debug, Clone)]
pub enum AuthPluginError {
    InvalidCredentials(String),
    Expired(String),
    NetworkError(String),
    Other(String),
}

impl std::fmt::Display for AuthPluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCredentials(msg) => write!(f, "Invalid credentials: {}", msg),
            Self::Expired(msg) => write!(f, "Token expired: {}", msg),
            Self::NetworkError(msg) => write!(f, "Network error: {}", msg),
            Self::Other(msg) => write!(f, "Auth error: {}", msg),
        }
    }
}

impl std::error::Error for AuthPluginError {}

// ─── Memory Plugin ───

/// Trait interface for memory/storage plugins.
pub trait MemoryPlugin: Send + Sync {
    /// Plugin name.
    fn name(&self) -> &str;
    /// Store a memory entry.
    fn store(&self, entry: &MemoryEntry) -> Result<String, MemoryPluginError>;
    /// Retrieve entries by query.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>, MemoryPluginError>;
    /// Delete an entry by ID.
    fn delete(&self, id: &str) -> Result<bool, MemoryPluginError>;
}

/// A memory entry for plugin storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Option<String>,
    pub content: String,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    pub embedding: Option<Vec<f32>>,
    pub timestamp: Option<i64>,
}

/// Memory plugin error.
#[derive(Debug, Clone)]
pub enum MemoryPluginError {
    StoreFailed(String),
    SearchFailed(String),
    NotFound(String),
    Other(String),
}

impl std::fmt::Display for MemoryPluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StoreFailed(msg) => write!(f, "Store failed: {}", msg),
            Self::SearchFailed(msg) => write!(f, "Search failed: {}", msg),
            Self::NotFound(msg) => write!(f, "Not found: {}", msg),
            Self::Other(msg) => write!(f, "Memory error: {}", msg),
        }
    }
}

impl std::error::Error for MemoryPluginError {}

// ─── Provider Plugin ───

/// Trait interface for LLM provider plugins.
pub trait ProviderPlugin: Send + Sync {
    /// Plugin name.
    fn name(&self) -> &str;
    /// Provider identifier (e.g., "openai", "custom-llm").
    fn provider_id(&self) -> &str;
    /// List available models.
    fn list_models(&self) -> Result<Vec<ProviderModel>, ProviderPluginError>;
    /// Supported features.
    fn capabilities(&self) -> ProviderCapabilities;
}

/// A model from a provider plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

/// Provider capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tool_use: bool,
    pub vision: bool,
    pub embeddings: bool,
}

/// Provider plugin error.
#[derive(Debug, Clone)]
pub enum ProviderPluginError {
    ConnectionFailed(String),
    AuthRequired(String),
    ModelNotFound(String),
    Other(String),
}

impl std::fmt::Display for ProviderPluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            Self::AuthRequired(msg) => write!(f, "Auth required: {}", msg),
            Self::ModelNotFound(msg) => write!(f, "Model not found: {}", msg),
            Self::Other(msg) => write!(f, "Provider error: {}", msg),
        }
    }
}

impl std::error::Error for ProviderPluginError {}

// ─── Plugin Registry ───

/// Tracks registered plugin implementations.
pub struct PluginRegistry {
    auth_plugins: Vec<String>,
    memory_plugins: Vec<String>,
    provider_plugins: Vec<String>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            auth_plugins: Vec::new(),
            memory_plugins: Vec::new(),
            provider_plugins: Vec::new(),
        }
    }

    pub fn register_auth(&mut self, name: String) {
        self.auth_plugins.push(name);
    }

    pub fn register_memory(&mut self, name: String) {
        self.memory_plugins.push(name);
    }

    pub fn register_provider(&mut self, name: String) {
        self.provider_plugins.push(name);
    }

    pub fn auth_plugins(&self) -> &[String] {
        &self.auth_plugins
    }

    pub fn memory_plugins(&self) -> &[String] {
        &self.memory_plugins
    }

    pub fn provider_plugins(&self) -> &[String] {
        &self.provider_plugins
    }

    pub fn total_count(&self) -> usize {
        self.auth_plugins.len() + self.memory_plugins.len() + self.provider_plugins.len()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_credentials() {
        let creds = AuthCredentials {
            provider: "openai".to_string(),
            username: None,
            api_key: Some("sk-test".to_string()),
            extra: std::collections::HashMap::new(),
        };
        assert_eq!(creds.provider, "openai");
    }

    #[test]
    fn test_auth_token() {
        let token = AuthToken {
            token: "abc123".to_string(),
            token_type: "Bearer".to_string(),
            expires_at: Some(9999999999),
            scopes: vec!["read".to_string()],
        };
        assert_eq!(token.token_type, "Bearer");
    }

    #[test]
    fn test_memory_entry() {
        let entry = MemoryEntry {
            id: Some("mem-1".to_string()),
            content: "test content".to_string(),
            metadata: std::collections::HashMap::new(),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            timestamp: None,
        };
        assert_eq!(entry.embedding.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_provider_capabilities() {
        let caps = ProviderCapabilities {
            streaming: true,
            tool_use: true,
            vision: false,
            embeddings: true,
        };
        assert!(caps.streaming);
        assert!(!caps.vision);
    }

    #[test]
    fn test_plugin_registry() {
        let mut reg = PluginRegistry::new();
        reg.register_auth("oauth-plugin".to_string());
        reg.register_memory("redis-store".to_string());
        reg.register_provider("custom-llm".to_string());
        assert_eq!(reg.total_count(), 3);
    }

    #[test]
    fn test_error_display() {
        let err = AuthPluginError::InvalidCredentials("bad key".to_string());
        assert!(format!("{}", err).contains("bad key"));

        let err = MemoryPluginError::StoreFailed("disk full".to_string());
        assert!(format!("{}", err).contains("disk full"));

        let err = ProviderPluginError::ConnectionFailed("timeout".to_string());
        assert!(format!("{}", err).contains("timeout"));
    }
}
