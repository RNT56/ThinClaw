//! Compatibility facade for MCP server configuration.
//!
//! Root-independent config DTOs, validation, and file persistence live in
//! `thinclaw-tools`. Database-backed loading stays here because it depends on
//! the root database abstraction.

use std::sync::Arc;

use async_trait::async_trait;
pub use thinclaw_tools::mcp::config::*;

/// Shared loader for reading persisted MCP server configuration.
#[derive(Clone)]
pub struct McpConfigStore {
    inner: thinclaw_tools::mcp::config::McpConfigStore,
}

#[derive(Clone)]
struct RootMcpConfigProvider {
    store: Option<Arc<dyn crate::db::Database>>,
    user_id: String,
}

impl McpConfigStore {
    pub fn new(store: Option<Arc<dyn crate::db::Database>>, user_id: impl Into<String>) -> Self {
        let provider = RootMcpConfigProvider {
            store,
            user_id: user_id.into(),
        };
        Self {
            inner: thinclaw_tools::mcp::config::McpConfigStore::new_provider(Arc::new(provider)),
        }
    }

    pub fn into_inner(self) -> thinclaw_tools::mcp::config::McpConfigStore {
        self.inner
    }

    pub async fn load_servers(&self) -> Result<McpServersFile, ConfigError> {
        self.inner.load_servers().await
    }

    pub async fn get_server(&self, name: &str) -> Result<Option<McpServerConfig>, ConfigError> {
        self.inner.get_server(name).await
    }

    pub async fn save_servers(&self, config: &McpServersFile) -> Result<(), ConfigError> {
        self.inner.save_servers(config).await
    }

    pub async fn upsert_server(&self, config: McpServerConfig) -> Result<(), ConfigError> {
        self.inner.upsert_server(config).await
    }

    pub async fn remove_server(&self, name: &str) -> Result<(), ConfigError> {
        self.inner.remove_server(name).await
    }
}

#[async_trait]
impl thinclaw_tools::mcp::config::McpConfigProvider for RootMcpConfigProvider {
    async fn load_servers(&self) -> Result<McpServersFile, ConfigError> {
        if let Some(ref store) = self.store {
            load_mcp_servers_from_db(store.as_ref(), &self.user_id).await
        } else {
            load_mcp_servers().await
        }
    }

    async fn save_servers(&self, config: &McpServersFile) -> Result<(), ConfigError> {
        if let Some(ref store) = self.store {
            save_mcp_servers_to_db(store.as_ref(), &self.user_id, config).await
        } else {
            save_mcp_servers(config).await
        }
    }
}

/// Load MCP server configurations from the database settings table.
///
/// Falls back to the disk file if DB has no entry.
pub async fn load_mcp_servers_from_db(
    store: &dyn crate::db::Database,
    user_id: &str,
) -> Result<McpServersFile, ConfigError> {
    match store.get_setting(user_id, "mcp_servers").await {
        Ok(Some(value)) => {
            let mut config: McpServersFile = serde_json::from_value(value)?;
            config.migrate_in_place();
            Ok(config)
        }
        Ok(None) => load_mcp_servers().await,
        Err(error) => {
            tracing::warn!(
                "Failed to load MCP servers from DB: {}, falling back to disk",
                error
            );
            load_mcp_servers().await
        }
    }
}

/// Save MCP server configurations to the database settings table.
pub async fn save_mcp_servers_to_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    config: &McpServersFile,
) -> Result<(), ConfigError> {
    let value = serde_json::to_value(config)?;
    store
        .set_setting(user_id, "mcp_servers", &value)
        .await
        .map_err(std::io::Error::other)?;
    Ok(())
}

/// Add a new MCP server configuration (DB-backed).
pub async fn add_mcp_server_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    config: McpServerConfig,
) -> Result<(), ConfigError> {
    config.validate()?;

    let mut servers = load_mcp_servers_from_db(store, user_id).await?;
    servers.upsert(config);
    save_mcp_servers_to_db(store, user_id, &servers).await?;

    Ok(())
}

/// Remove an MCP server by name (DB-backed).
pub async fn remove_mcp_server_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    name: &str,
) -> Result<(), ConfigError> {
    let mut servers = load_mcp_servers_from_db(store, user_id).await?;

    if !servers.remove(name) {
        return Err(ConfigError::ServerNotFound {
            name: name.to_string(),
        });
    }

    save_mcp_servers_to_db(store, user_id, &servers).await?;
    Ok(())
}
