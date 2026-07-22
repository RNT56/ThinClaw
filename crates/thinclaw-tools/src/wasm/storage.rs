//! WASM binary storage with integrity verification.
//!
//! Stores compiled WASM tools in PostgreSQL with BLAKE3 hash verification.
//! On load, the hash is verified to detect tampering.
//!
//! # Storage Flow
//!
//! ```text
//! WASM bytes ──► BLAKE3 hash ──► Store in PostgreSQL
//!                    │               (binary + hash)
//!                    │
//!                    └──► Later: Load ──► Verify hash ──► Return bytes
//! ```

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use deadpool_postgres::Pool;
use uuid::Uuid;

use crate::wasm::capabilities::{Capabilities, EndpointPattern};

/// Trust level for a WASM tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Built-in system tool (highest trust).
    System,
    /// Audited and verified tool.
    Verified,
    /// User-uploaded tool (untrusted).
    User,
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrustLevel::System => write!(f, "system"),
            TrustLevel::Verified => write!(f, "verified"),
            TrustLevel::User => write!(f, "user"),
        }
    }
}

impl std::str::FromStr for TrustLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "system" => Ok(TrustLevel::System),
            "verified" => Ok(TrustLevel::Verified),
            "user" => Ok(TrustLevel::User),
            _ => Err(format!("Unknown trust level: {}", s)),
        }
    }
}

/// Status of a WASM tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// Tool is active and can be used.
    Active,
    /// Tool is disabled (manually or due to errors).
    Disabled,
    /// Tool is quarantined (suspected malicious).
    Quarantined,
}

impl std::fmt::Display for ToolStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolStatus::Active => write!(f, "active"),
            ToolStatus::Disabled => write!(f, "disabled"),
            ToolStatus::Quarantined => write!(f, "quarantined"),
        }
    }
}

impl std::str::FromStr for ToolStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(ToolStatus::Active),
            "disabled" => Ok(ToolStatus::Disabled),
            "quarantined" => Ok(ToolStatus::Quarantined),
            _ => Err(format!("Unknown status: {}", s)),
        }
    }
}

/// A stored WASM tool.
#[derive(Debug, Clone)]
pub struct StoredWasmTool {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    pub source_url: Option<String>,
    pub trust_level: TrustLevel,
    pub status: ToolStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full tool data including binary (not returned by default for efficiency).
#[derive(Debug)]
pub struct StoredWasmToolWithBinary {
    pub tool: StoredWasmTool,
    pub wasm_binary: Vec<u8>,
    pub binary_hash: Vec<u8>,
}

/// Capabilities stored in the database.
#[derive(Debug, Clone)]
pub struct StoredCapabilities {
    pub id: Uuid,
    pub wasm_tool_id: Uuid,
    pub http_allowlist: Vec<EndpointPattern>,
    pub allowed_secrets: Vec<String>,
    pub tool_aliases: HashMap<String, String>,
    pub requests_per_minute: u32,
    pub requests_per_hour: u32,
    pub max_request_body_bytes: i64,
    pub max_response_body_bytes: i64,
    pub workspace_read_prefixes: Vec<String>,
    pub http_timeout_secs: i32,
}

impl StoredCapabilities {
    /// Convert to runtime Capabilities struct.
    pub fn to_capabilities(&self) -> Result<Capabilities, WasmStorageError> {
        use crate::wasm::capabilities_schema::{
            CapabilitiesFile, EndpointPatternSchema, HttpCapabilitySchema, RateLimitSchema,
            SecretsCapabilitySchema, ToolInvokeCapabilitySchema, WorkspaceCapabilitySchema,
        };

        let max_request_bytes = usize::try_from(self.max_request_body_bytes).map_err(|_| {
            WasmStorageError::InvalidData(
                "stored maximum request size is negative or unsupported".to_string(),
            )
        })?;
        let max_response_bytes = usize::try_from(self.max_response_body_bytes).map_err(|_| {
            WasmStorageError::InvalidData(
                "stored maximum response size is negative or unsupported".to_string(),
            )
        })?;
        let timeout_secs = u64::try_from(self.http_timeout_secs).map_err(|_| {
            WasmStorageError::InvalidData("stored HTTP timeout is negative".to_string())
        })?;
        let rate_limit = RateLimitSchema {
            requests_per_minute: self.requests_per_minute,
            requests_per_hour: self.requests_per_hour,
        };
        let schema = CapabilitiesFile {
            http: (!self.http_allowlist.is_empty()).then(|| HttpCapabilitySchema {
                allowlist: self
                    .http_allowlist
                    .iter()
                    .map(|endpoint| EndpointPatternSchema {
                        host: endpoint.host.clone(),
                        path_prefix: endpoint.path_prefix.clone(),
                        methods: endpoint.methods.clone(),
                    })
                    .collect(),
                credentials: HashMap::new(),
                rate_limit: Some(rate_limit.clone()),
                max_request_bytes: Some(max_request_bytes),
                max_response_bytes: Some(max_response_bytes),
                timeout_secs: Some(timeout_secs),
            }),
            secrets: (!self.allowed_secrets.is_empty()).then(|| SecretsCapabilitySchema {
                allowed_names: self.allowed_secrets.clone(),
            }),
            tool_invoke: (!self.tool_aliases.is_empty()).then(|| ToolInvokeCapabilitySchema {
                aliases: self.tool_aliases.clone(),
                rate_limit: Some(rate_limit),
            }),
            workspace: (!self.workspace_read_prefixes.is_empty()).then(|| {
                WorkspaceCapabilitySchema {
                    allowed_prefixes: self.workspace_read_prefixes.clone(),
                }
            }),
            ..Default::default()
        };
        schema.validate().map_err(WasmStorageError::InvalidData)?;
        Ok(schema.to_capabilities())
    }
}

/// Error from WASM storage operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum WasmStorageError {
    #[error("Tool not found: {0}")]
    NotFound(String),

    #[error("Tool is disabled")]
    Disabled,

    #[error("Tool is quarantined")]
    Quarantined,

    #[error("Binary integrity check failed: hash mismatch")]
    IntegrityCheckFailed,

    #[error("Database error: {0}")]
    Database(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

/// Trait for WASM tool storage.
#[async_trait]
pub trait WasmToolStore: Send + Sync {
    /// Store a new WASM tool.
    async fn store(&self, params: StoreToolParams) -> Result<StoredWasmTool, WasmStorageError>;

    /// Get tool metadata (without binary).
    async fn get(&self, user_id: &str, name: &str) -> Result<StoredWasmTool, WasmStorageError>;

    /// Get tool with binary (verifies integrity).
    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmToolWithBinary, WasmStorageError>;

    /// Get tool capabilities.
    async fn get_capabilities(
        &self,
        user_id: &str,
        tool_id: Uuid,
    ) -> Result<Option<StoredCapabilities>, WasmStorageError>;

    /// List all tools for a user.
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError>;

    /// Update tool status.
    async fn update_status(
        &self,
        user_id: &str,
        name: &str,
        status: ToolStatus,
    ) -> Result<(), WasmStorageError>;

    /// Delete a tool.
    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmStorageError>;
}

/// Parameters for storing a new tool.
pub struct StoreToolParams {
    pub user_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub wasm_binary: Vec<u8>,
    pub parameters_schema: serde_json::Value,
    pub source_url: Option<String>,
    pub trust_level: TrustLevel,
}

#[cfg(any(feature = "postgres", feature = "libsql", test))]
fn validate_store_tool_params(params: &StoreToolParams) -> Result<(), WasmStorageError> {
    let valid_name = !params.name.is_empty()
        && params.name.len() <= 128
        && params
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    let valid_version = !params.version.is_empty()
        && params.version.len() <= 64
        && params
            .version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'));
    let valid_description = params.description.len() <= 64 * 1024
        && !params
            .description
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'));
    let valid_user = !params.user_id.trim().is_empty()
        && params.user_id.len() <= 256
        && !params.user_id.chars().any(char::is_control);
    let valid_wasm = (8..=64 * 1024 * 1024).contains(&params.wasm_binary.len())
        && params.wasm_binary.starts_with(b"\0asm");
    let valid_schema = serde_json::to_vec(&params.parameters_schema)
        .map(|bytes| bytes.len() <= 1024 * 1024)
        .unwrap_or(false);
    let valid_source = params.source_url.as_deref().is_none_or(|source| {
        source.len() <= 4096
            && url::Url::parse(source).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none()
            })
    });
    if !valid_name
        || !valid_version
        || !valid_description
        || !valid_user
        || !valid_wasm
        || !valid_schema
        || !valid_source
    {
        return Err(WasmStorageError::InvalidData(
            "WASM tool metadata, binary, schema, or source URL is malformed or oversized"
                .to_string(),
        ));
    }
    Ok(())
}

/// Compute BLAKE3 hash of WASM binary.
pub fn compute_binary_hash(binary: &[u8]) -> Vec<u8> {
    let hash = blake3::hash(binary);
    hash.as_bytes().to_vec()
}

/// Verify binary integrity against stored hash.
pub fn verify_binary_integrity(binary: &[u8], expected_hash: &[u8]) -> bool {
    let actual_hash = compute_binary_hash(binary);
    actual_hash == expected_hash
}

/// PostgreSQL implementation of WasmToolStore.
#[cfg(feature = "postgres")]
pub struct PostgresWasmToolStore {
    pool: Pool,
}

#[cfg(feature = "postgres")]
impl PostgresWasmToolStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl WasmToolStore for PostgresWasmToolStore {
    async fn store(&self, params: StoreToolParams) -> Result<StoredWasmTool, WasmStorageError> {
        validate_store_tool_params(&params)?;
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let binary_hash = compute_binary_hash(&params.wasm_binary);
        let id = Uuid::new_v4();
        let now = Utc::now();

        let row = client
            .query_one(
                r#"
                INSERT INTO wasm_tools (
                    id, user_id, name, version, description, wasm_binary, binary_hash,
                    parameters_schema, source_url, trust_level, status, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'active', $11, $11)
                ON CONFLICT (user_id, name, version) DO UPDATE SET
                    description = EXCLUDED.description,
                    wasm_binary = EXCLUDED.wasm_binary,
                    binary_hash = EXCLUDED.binary_hash,
                    parameters_schema = EXCLUDED.parameters_schema,
                    source_url = EXCLUDED.source_url,
                    updated_at = NOW()
                RETURNING id, user_id, name, version, description, parameters_schema,
                          source_url, trust_level, status, created_at, updated_at
                "#,
                &[
                    &id,
                    &params.user_id,
                    &params.name,
                    &params.version,
                    &params.description,
                    &params.wasm_binary,
                    &binary_hash,
                    &params.parameters_schema,
                    &params.source_url,
                    &params.trust_level.to_string(),
                    &now,
                ],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        row_to_tool(&row)
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<StoredWasmTool, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = $1 AND name = $2 AND status = 'active'
                  AND octet_length(wasm_binary) BETWEEN 8 AND 67108864
                ORDER BY updated_at DESC, created_at DESC, id DESC
                LIMIT 1
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let tool = row_to_tool(&r)?;
                match tool.status {
                    ToolStatus::Active => Ok(tool),
                    ToolStatus::Disabled => Err(WasmStorageError::Disabled),
                    ToolStatus::Quarantined => Err(WasmStorageError::Quarantined),
                }
            }
            None => Err(WasmStorageError::NotFound(name.to_string())),
        }
    }

    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmToolWithBinary, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT id, user_id, name, version, description, wasm_binary, binary_hash,
                       parameters_schema, source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = $1 AND name = $2 AND status = 'active'
                  AND octet_length(wasm_binary) BETWEEN 8 AND 67108864
                ORDER BY updated_at DESC, created_at DESC, id DESC
                LIMIT 1
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let wasm_binary: Vec<u8> = r
                    .try_get("wasm_binary")
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let binary_hash: Vec<u8> = r
                    .try_get("binary_hash")
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;

                if wasm_binary.len() < 8
                    || wasm_binary.len() > 64 * 1024 * 1024
                    || !wasm_binary.starts_with(b"\0asm")
                    || binary_hash.len() != 32
                {
                    return Err(WasmStorageError::InvalidData(
                        "stored WASM binary or integrity hash is malformed".to_string(),
                    ));
                }

                // Verify integrity
                if !verify_binary_integrity(&wasm_binary, &binary_hash) {
                    tracing::error!(
                        user_id = user_id,
                        name = name,
                        "WASM binary integrity check failed"
                    );
                    return Err(WasmStorageError::IntegrityCheckFailed);
                }

                let tool = row_to_tool(&r)?;

                match tool.status {
                    ToolStatus::Active => Ok(StoredWasmToolWithBinary {
                        tool,
                        wasm_binary,
                        binary_hash,
                    }),
                    ToolStatus::Disabled => Err(WasmStorageError::Disabled),
                    ToolStatus::Quarantined => Err(WasmStorageError::Quarantined),
                }
            }
            None => Err(WasmStorageError::NotFound(name.to_string())),
        }
    }

    async fn get_capabilities(
        &self,
        user_id: &str,
        tool_id: Uuid,
    ) -> Result<Option<StoredCapabilities>, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT capabilities.id, capabilities.wasm_tool_id,
                       capabilities.http_allowlist, capabilities.allowed_secrets,
                       capabilities.tool_aliases, capabilities.requests_per_minute,
                       capabilities.requests_per_hour, capabilities.max_request_body_bytes,
                       capabilities.max_response_body_bytes,
                       capabilities.workspace_read_prefixes, capabilities.http_timeout_secs
                FROM tool_capabilities AS capabilities
                INNER JOIN wasm_tools AS tool ON tool.id = capabilities.wasm_tool_id
                WHERE capabilities.wasm_tool_id = $1 AND tool.user_id = $2
                "#,
                &[&tool_id, &user_id],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let database_error =
                    |error: tokio_postgres::Error| WasmStorageError::Database(error.to_string());
                let http_allowlist_json: serde_json::Value =
                    r.try_get("http_allowlist").map_err(database_error)?;
                let tool_aliases_json: serde_json::Value =
                    r.try_get("tool_aliases").map_err(database_error)?;

                let http_allowlist: Vec<EndpointPattern> =
                    serde_json::from_value(http_allowlist_json).map_err(|error| {
                        WasmStorageError::InvalidData(format!(
                            "stored HTTP allowlist is invalid: {error}"
                        ))
                    })?;
                let tool_aliases: HashMap<String, String> =
                    serde_json::from_value(tool_aliases_json).map_err(|error| {
                        WasmStorageError::InvalidData(format!(
                            "stored tool aliases are invalid: {error}"
                        ))
                    })?;
                let requests_per_minute = u32::try_from(
                    r.try_get::<_, i32>("requests_per_minute")
                        .map_err(database_error)?,
                )
                .map_err(|_| {
                    WasmStorageError::InvalidData(
                        "stored requests-per-minute value is negative".to_string(),
                    )
                })?;
                let requests_per_hour = u32::try_from(
                    r.try_get::<_, i32>("requests_per_hour")
                        .map_err(database_error)?,
                )
                .map_err(|_| {
                    WasmStorageError::InvalidData(
                        "stored requests-per-hour value is negative".to_string(),
                    )
                })?;

                Ok(Some(StoredCapabilities {
                    id: r.try_get("id").map_err(database_error)?,
                    wasm_tool_id: r.try_get("wasm_tool_id").map_err(database_error)?,
                    http_allowlist,
                    allowed_secrets: r.try_get("allowed_secrets").map_err(database_error)?,
                    tool_aliases,
                    requests_per_minute,
                    requests_per_hour,
                    max_request_body_bytes: r
                        .try_get("max_request_body_bytes")
                        .map_err(database_error)?,
                    max_response_body_bytes: r
                        .try_get("max_response_body_bytes")
                        .map_err(database_error)?,
                    workspace_read_prefixes: r
                        .try_get("workspace_read_prefixes")
                        .map_err(database_error)?,
                    http_timeout_secs: r.try_get("http_timeout_secs").map_err(database_error)?,
                }))
            }
            None => Ok(None),
        }
    }

    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let rows = client
            .query(
                r#"
                SELECT DISTINCT ON (name) id, user_id, name, version, description,
                       parameters_schema, source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = $1
                ORDER BY name, updated_at DESC, created_at DESC, id DESC
                LIMIT 257
                "#,
                &[&user_id],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        rows.into_iter().map(|r| row_to_tool(&r)).collect()
    }

    async fn update_status(
        &self,
        user_id: &str,
        name: &str,
        status: ToolStatus,
    ) -> Result<(), WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let result = client
            .execute(
                "UPDATE wasm_tools SET status = $1, updated_at = NOW() WHERE user_id = $2 AND name = $3",
                &[&status.to_string(), &user_id, &name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        if result == 0 {
            return Err(WasmStorageError::NotFound(name.to_string()));
        }

        Ok(())
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let result = client
            .execute(
                "DELETE FROM wasm_tools WHERE user_id = $1 AND name = $2",
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        Ok(result > 0)
    }
}

#[cfg(feature = "postgres")]
fn row_to_tool(row: &tokio_postgres::Row) -> Result<StoredWasmTool, WasmStorageError> {
    let database_error =
        |error: tokio_postgres::Error| WasmStorageError::Database(error.to_string());
    let trust_level_str: String = row.try_get("trust_level").map_err(database_error)?;
    let status_str: String = row.try_get("status").map_err(database_error)?;

    Ok(StoredWasmTool {
        id: row.try_get("id").map_err(database_error)?,
        user_id: row.try_get("user_id").map_err(database_error)?,
        name: row.try_get("name").map_err(database_error)?,
        version: row.try_get("version").map_err(database_error)?,
        description: row.try_get("description").map_err(database_error)?,
        parameters_schema: row.try_get("parameters_schema").map_err(database_error)?,
        source_url: row.try_get("source_url").map_err(database_error)?,
        trust_level: trust_level_str
            .parse()
            .map_err(WasmStorageError::InvalidData)?,
        status: status_str.parse().map_err(WasmStorageError::InvalidData)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

// ==================== libSQL implementation ====================

/// libSQL/Turso implementation of WasmToolStore.
///
/// Holds an `Arc<Database>` handle and creates a fresh connection per operation,
/// matching the connection-per-request pattern used by the main `LibSqlBackend`.
#[cfg(feature = "libsql")]
pub struct LibSqlWasmToolStore {
    db: std::sync::Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlWasmToolStore {
    pub fn new(db: std::sync::Arc<libsql::Database>) -> Self {
        Self { db }
    }

    async fn connect(&self) -> Result<libsql::Connection, WasmStorageError> {
        let conn = self
            .db
            .connect()
            .map_err(|e| WasmStorageError::Database(format!("Connection failed: {}", e)))?;
        let mut rows = conn
            .query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|e| {
                WasmStorageError::Database(format!("Failed to set busy_timeout: {}", e))
            })?;
        let _ = rows.next().await.map_err(|e| {
            WasmStorageError::Database(format!("Failed to confirm busy_timeout: {}", e))
        })?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl WasmToolStore for LibSqlWasmToolStore {
    async fn store(&self, params: StoreToolParams) -> Result<StoredWasmTool, WasmStorageError> {
        validate_store_tool_params(&params)?;
        let binary_hash = compute_binary_hash(&params.wasm_binary);
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let schema_str = serde_json::to_string(&params.parameters_schema)
            .map_err(|e| WasmStorageError::InvalidData(e.to_string()))?;

        // Wrap INSERT + read-back in a transaction to prevent TOCTOU races
        let conn = self.connect().await?;
        let tx = conn
            .transaction()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        tx.execute(
            r#"
                INSERT INTO wasm_tools (
                    id, user_id, name, version, description, wasm_binary, binary_hash,
                    parameters_schema, source_url, trust_level, status, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, ?11)
                ON CONFLICT (user_id, name, version) DO UPDATE SET
                    description = excluded.description,
                    wasm_binary = excluded.wasm_binary,
                    binary_hash = excluded.binary_hash,
                    parameters_schema = excluded.parameters_schema,
                    source_url = excluded.source_url,
                    updated_at = ?11
                "#,
            libsql::params![
                id.to_string(),
                params.user_id.as_str(),
                params.name.as_str(),
                params.version.as_str(),
                params.description.as_str(),
                libsql::Value::Blob(params.wasm_binary),
                libsql::Value::Blob(binary_hash),
                schema_str.as_str(),
                libsql_wasm_opt_text(params.source_url.as_deref()),
                params.trust_level.to_string(),
                now.as_str(),
            ],
        )
        .await
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        // Read back the row within the same transaction
        let mut rows = tx
            .query(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1 AND name = ?2
                ORDER BY updated_at DESC, created_at DESC, rowid DESC
                LIMIT 1
                "#,
                libsql::params![params.user_id.as_str(), params.name.as_str()],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
            .ok_or_else(|| {
                WasmStorageError::Database("Insert succeeded but row not found".into())
            })?;

        let tool = libsql_row_to_tool(&row)?;

        tx.commit()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        Ok(tool)
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<StoredWasmTool, WasmStorageError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1 AND name = ?2 AND status = 'active'
                  AND length(wasm_binary) BETWEEN 8 AND 67108864
                ORDER BY updated_at DESC, created_at DESC, rowid DESC
                LIMIT 1
                "#,
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            Some(row) => {
                let tool = libsql_row_to_tool(&row)?;
                match tool.status {
                    ToolStatus::Active => Ok(tool),
                    ToolStatus::Disabled => Err(WasmStorageError::Disabled),
                    ToolStatus::Quarantined => Err(WasmStorageError::Quarantined),
                }
            }
            None => Err(WasmStorageError::NotFound(name.to_string())),
        }
    }

    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmToolWithBinary, WasmStorageError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, description, wasm_binary, binary_hash,
                       parameters_schema, source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1 AND name = ?2 AND status = 'active'
                  AND length(wasm_binary) BETWEEN 8 AND 67108864
                ORDER BY updated_at DESC, created_at DESC, rowid DESC
                LIMIT 1
                "#,
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            Some(row) => {
                let wasm_binary: Vec<u8> = row
                    .get(5)
                    .map_err(|e| WasmStorageError::Database(e.to_string()))?;
                let binary_hash: Vec<u8> = row
                    .get(6)
                    .map_err(|e| WasmStorageError::Database(e.to_string()))?;

                if wasm_binary.len() < 8
                    || wasm_binary.len() > 64 * 1024 * 1024
                    || !wasm_binary.starts_with(b"\0asm")
                    || binary_hash.len() != 32
                {
                    return Err(WasmStorageError::InvalidData(
                        "stored WASM binary or integrity hash is malformed".to_string(),
                    ));
                }

                if !verify_binary_integrity(&wasm_binary, &binary_hash) {
                    tracing::error!(
                        user_id = user_id,
                        name = name,
                        "WASM binary integrity check failed"
                    );
                    return Err(WasmStorageError::IntegrityCheckFailed);
                }

                // Parse metadata from the row (different column offsets due to binary/hash)
                let tool = libsql_row_to_tool_with_offset(&row)?;

                match tool.status {
                    ToolStatus::Active => Ok(StoredWasmToolWithBinary {
                        tool,
                        wasm_binary,
                        binary_hash,
                    }),
                    ToolStatus::Disabled => Err(WasmStorageError::Disabled),
                    ToolStatus::Quarantined => Err(WasmStorageError::Quarantined),
                }
            }
            None => Err(WasmStorageError::NotFound(name.to_string())),
        }
    }

    async fn get_capabilities(
        &self,
        user_id: &str,
        tool_id: Uuid,
    ) -> Result<Option<StoredCapabilities>, WasmStorageError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT capabilities.id, capabilities.wasm_tool_id,
                       capabilities.http_allowlist, capabilities.allowed_secrets,
                       capabilities.tool_aliases, capabilities.requests_per_minute,
                       capabilities.requests_per_hour, capabilities.max_request_body_bytes,
                       capabilities.max_response_body_bytes,
                       capabilities.workspace_read_prefixes, capabilities.http_timeout_secs
                FROM tool_capabilities AS capabilities
                INNER JOIN wasm_tools AS tool ON tool.id = capabilities.wasm_tool_id
                WHERE capabilities.wasm_tool_id = ?1 AND tool.user_id = ?2
                "#,
                libsql::params![tool_id.to_string(), user_id],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            Some(row) => {
                let id_str: String = row
                    .get(0)
                    .map_err(|e| WasmStorageError::Database(e.to_string()))?;
                let tool_id_str: String = row
                    .get(1)
                    .map_err(|e| WasmStorageError::Database(e.to_string()))?;
                let http_allowlist_str: String = row
                    .get(2)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let allowed_secrets_str: String = row
                    .get(3)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let tool_aliases_str: String = row
                    .get(4)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let rpm: i64 = row
                    .get(5)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let rph: i64 = row
                    .get(6)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let max_req: i64 = row
                    .get(7)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let max_resp: i64 = row
                    .get(8)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let ws_prefixes_str: String = row
                    .get(9)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;
                let timeout: i64 = row
                    .get(10)
                    .map_err(|error| WasmStorageError::Database(error.to_string()))?;

                let http_allowlist: Vec<EndpointPattern> =
                    serde_json::from_str(&http_allowlist_str).map_err(|error| {
                        WasmStorageError::InvalidData(format!(
                            "stored HTTP allowlist is invalid: {error}"
                        ))
                    })?;
                let allowed_secrets: Vec<String> = serde_json::from_str(&allowed_secrets_str)
                    .map_err(|error| {
                        WasmStorageError::InvalidData(format!(
                            "stored secret allowlist is invalid: {error}"
                        ))
                    })?;
                let tool_aliases: HashMap<String, String> = serde_json::from_str(&tool_aliases_str)
                    .map_err(|error| {
                        WasmStorageError::InvalidData(format!(
                            "stored tool aliases are invalid: {error}"
                        ))
                    })?;
                let workspace_read_prefixes: Vec<String> = serde_json::from_str(&ws_prefixes_str)
                    .map_err(|error| {
                    WasmStorageError::InvalidData(format!(
                        "stored workspace prefixes are invalid: {error}"
                    ))
                })?;
                let requests_per_minute = u32::try_from(rpm).map_err(|_| {
                    WasmStorageError::InvalidData(
                        "stored requests-per-minute value is negative".to_string(),
                    )
                })?;
                let requests_per_hour = u32::try_from(rph).map_err(|_| {
                    WasmStorageError::InvalidData(
                        "stored requests-per-hour value is negative".to_string(),
                    )
                })?;
                let http_timeout_secs = i32::try_from(timeout).map_err(|_| {
                    WasmStorageError::InvalidData(
                        "stored HTTP timeout exceeds the supported range".to_string(),
                    )
                })?;

                Ok(Some(StoredCapabilities {
                    id: id_str
                        .parse()
                        .map_err(|e: uuid::Error| WasmStorageError::InvalidData(e.to_string()))?,
                    wasm_tool_id: tool_id_str
                        .parse()
                        .map_err(|e: uuid::Error| WasmStorageError::InvalidData(e.to_string()))?,
                    http_allowlist,
                    allowed_secrets,
                    tool_aliases,
                    requests_per_minute,
                    requests_per_hour,
                    max_request_body_bytes: max_req,
                    max_response_body_bytes: max_resp,
                    workspace_read_prefixes,
                    http_timeout_secs,
                }))
            }
            None => Ok(None),
        }
    }

    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError> {
        // SQLite doesn't have DISTINCT ON, so use row insertion order to get
        // the most recently stored generation for each name.
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1
                  AND rowid = (
                      SELECT newer.rowid
                      FROM wasm_tools AS newer
                      WHERE newer.user_id = wasm_tools.user_id
                        AND newer.name = wasm_tools.name
                      ORDER BY newer.updated_at DESC, newer.created_at DESC, newer.rowid DESC
                      LIMIT 1
                  )
                ORDER BY name
                LIMIT 257
                "#,
                libsql::params![user_id],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let mut tools = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            tools.push(libsql_row_to_tool(&row)?);
        }
        Ok(tools)
    }

    async fn update_status(
        &self,
        user_id: &str,
        name: &str,
        status: ToolStatus,
    ) -> Result<(), WasmStorageError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let conn = self.connect().await?;

        let result = conn
            .execute(
                "UPDATE wasm_tools SET status = ?1, updated_at = ?2 WHERE user_id = ?3 AND name = ?4",
                libsql::params![status.to_string(), now.as_str(), user_id, name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        if result == 0 {
            return Err(WasmStorageError::NotFound(name.to_string()));
        }

        Ok(())
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmStorageError> {
        let conn = self.connect().await?;
        let result = conn
            .execute(
                "DELETE FROM wasm_tools WHERE user_id = ?1 AND name = ?2",
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        Ok(result > 0)
    }
}

#[cfg(feature = "libsql")]
fn libsql_wasm_opt_text(s: Option<&str>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s.to_string()),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn libsql_wasm_parse_ts(s: &str) -> Result<DateTime<Utc>, WasmStorageError> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(ndt.and_utc());
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(WasmStorageError::InvalidData(format!(
        "unparseable timestamp: {:?}",
        s
    )))
}

/// Parse a tool row with standard column order (no binary columns).
/// Columns: id(0), user_id(1), name(2), version(3), description(4),
///          parameters_schema(5), source_url(6), trust_level(7), status(8),
///          created_at(9), updated_at(10)
#[cfg(feature = "libsql")]
fn libsql_row_to_tool(row: &libsql::Row) -> Result<StoredWasmTool, WasmStorageError> {
    libsql_row_to_tool_at(row, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
}

/// Parse a tool row when binary columns are present (get_with_binary query).
/// Columns: id(0), user_id(1), name(2), version(3), description(4),
///          wasm_binary(5), binary_hash(6),
///          parameters_schema(7), source_url(8), trust_level(9), status(10),
///          created_at(11), updated_at(12)
#[cfg(feature = "libsql")]
fn libsql_row_to_tool_with_offset(row: &libsql::Row) -> Result<StoredWasmTool, WasmStorageError> {
    libsql_row_to_tool_at(row, 0, 1, 2, 3, 4, 7, 8, 9, 10, 11, 12)
}

#[cfg(feature = "libsql")]
#[allow(clippy::too_many_arguments)]
fn libsql_row_to_tool_at(
    row: &libsql::Row,
    id_idx: i32,
    user_id_idx: i32,
    name_idx: i32,
    version_idx: i32,
    description_idx: i32,
    schema_idx: i32,
    source_url_idx: i32,
    trust_level_idx: i32,
    status_idx: i32,
    created_at_idx: i32,
    updated_at_idx: i32,
) -> Result<StoredWasmTool, WasmStorageError> {
    let id_str: String = row
        .get(id_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let trust_level_str: String = row
        .get(trust_level_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let status_str: String = row
        .get(status_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let schema_str: String = row
        .get(schema_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let created_at_str: String = row
        .get(created_at_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let updated_at_str: String = row
        .get(updated_at_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;

    Ok(StoredWasmTool {
        id: id_str
            .parse()
            .map_err(|e: uuid::Error| WasmStorageError::InvalidData(e.to_string()))?,
        user_id: row
            .get(user_id_idx)
            .map_err(|e| WasmStorageError::Database(e.to_string()))?,
        name: row
            .get(name_idx)
            .map_err(|e| WasmStorageError::Database(e.to_string()))?,
        version: row
            .get(version_idx)
            .map_err(|e| WasmStorageError::Database(e.to_string()))?,
        description: row
            .get(description_idx)
            .map_err(|e| WasmStorageError::Database(e.to_string()))?,
        parameters_schema: serde_json::from_str(&schema_str).map_err(|error| {
            WasmStorageError::InvalidData(format!("stored parameter schema is invalid: {error}"))
        })?,
        source_url: row
            .get::<Option<String>>(source_url_idx)
            .map_err(|error| WasmStorageError::Database(error.to_string()))?
            .filter(|source| !source.is_empty()),
        trust_level: trust_level_str
            .parse()
            .map_err(WasmStorageError::InvalidData)?,
        status: status_str.parse().map_err(WasmStorageError::InvalidData)?,
        created_at: libsql_wasm_parse_ts(&created_at_str)?,
        updated_at: libsql_wasm_parse_ts(&updated_at_str)?,
    })
}

#[cfg(test)]
mod tests {
    use crate::wasm::storage::{
        StoreToolParams, StoredCapabilities, ToolStatus, TrustLevel, compute_binary_hash,
        validate_store_tool_params, verify_binary_integrity,
    };

    #[test]
    fn test_compute_hash() {
        let binary = b"(module)";
        let hash = compute_binary_hash(binary);
        assert_eq!(hash.len(), 32); // BLAKE3 produces 32-byte hash
    }

    #[test]
    fn test_verify_integrity_success() {
        let binary = b"test wasm binary content";
        let hash = compute_binary_hash(binary);
        assert!(verify_binary_integrity(binary, &hash));
    }

    #[test]
    fn test_verify_integrity_failure() {
        let binary = b"test wasm binary content";
        let hash = compute_binary_hash(binary);
        let tampered = b"tampered wasm binary content";
        assert!(!verify_binary_integrity(tampered, &hash));
    }

    #[test]
    fn test_trust_level_parse() {
        assert_eq!("system".parse::<TrustLevel>().unwrap(), TrustLevel::System);
        assert_eq!(
            "verified".parse::<TrustLevel>().unwrap(),
            TrustLevel::Verified
        );
        assert_eq!("user".parse::<TrustLevel>().unwrap(), TrustLevel::User);
        assert!("invalid".parse::<TrustLevel>().is_err());
    }

    #[test]
    fn test_status_parse() {
        assert_eq!("active".parse::<ToolStatus>().unwrap(), ToolStatus::Active);
        assert_eq!(
            "disabled".parse::<ToolStatus>().unwrap(),
            ToolStatus::Disabled
        );
        assert_eq!(
            "quarantined".parse::<ToolStatus>().unwrap(),
            ToolStatus::Quarantined
        );
        assert!("invalid".parse::<ToolStatus>().is_err());
    }

    #[test]
    fn stored_capabilities_reject_negative_numeric_values() {
        let stored = StoredCapabilities {
            id: uuid::Uuid::new_v4(),
            wasm_tool_id: uuid::Uuid::new_v4(),
            http_allowlist: vec![crate::wasm::EndpointPattern::host("api.example.com")],
            allowed_secrets: Vec::new(),
            tool_aliases: std::collections::HashMap::new(),
            requests_per_minute: 60,
            requests_per_hour: 1000,
            max_request_body_bytes: -1,
            max_response_body_bytes: 1024,
            workspace_read_prefixes: Vec::new(),
            http_timeout_secs: 30,
        };

        assert!(stored.to_capabilities().is_err());
    }

    #[test]
    fn store_params_require_bounded_valid_wasm() {
        let mut params = StoreToolParams {
            user_id: "user".to_string(),
            name: "example".to_string(),
            version: "1.0.0".to_string(),
            description: "Example".to_string(),
            wasm_binary: b"\0asm\x01\0\0\0".to_vec(),
            parameters_schema: serde_json::json!({"type": "object"}),
            source_url: Some("https://example.com/tool.wasm".to_string()),
            trust_level: TrustLevel::User,
        };
        assert!(validate_store_tool_params(&params).is_ok());

        params.name = "../escape".to_string();
        assert!(validate_store_tool_params(&params).is_err());
    }
}
