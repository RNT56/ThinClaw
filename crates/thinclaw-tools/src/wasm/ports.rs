//! Narrow host ports used by the root-independent WASM runtime.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thinclaw_secrets::{
    CreateSecretParams, DecryptedSecret, SecretAccessContext, SecretError, SecretsStore,
};
use thinclaw_tools_core::Tool;
use thinclaw_types::JobContext;

use crate::wasm::{
    Capabilities, OAuthRefreshConfig, ResourceLimits, WasmError, WasmStorageError, WasmToolRuntime,
    WasmToolStore,
};

/// Policy-aware bridge for WASM `tool_invoke` host calls.
#[async_trait]
pub trait HostToolInvoker: Send + Sync {
    async fn invoke_json(
        &self,
        job_ctx: &JobContext,
        tool_name: &str,
        params_json: &str,
    ) -> Result<String, String>;
}

/// Secret lookup operations needed by the WASM host boundary.
#[async_trait]
pub trait SecretResolver: Send + Sync {
    async fn get_secret_expiry(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<DateTime<Utc>>, SecretError>;

    async fn get_for_injection(
        &self,
        user_id: &str,
        name: &str,
        context: SecretAccessContext,
    ) -> Result<DecryptedSecret, SecretError>;

    async fn list_secret_names(&self, user_id: &str) -> Result<Vec<String>, SecretError>;

    async fn create_secret(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<(), SecretError>;

    async fn record_leak_detection_event(
        &self,
        user_id: &str,
        source: &str,
        action_taken: &str,
        content_hash: &str,
        redacted_preview: Option<&str>,
    ) -> Result<(), SecretError>;
}

#[async_trait]
impl<T> SecretResolver for T
where
    T: SecretsStore + Send + Sync + ?Sized,
{
    async fn get_secret_expiry(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<DateTime<Utc>>, SecretError> {
        self.get(user_id, name)
            .await
            .map(|secret| secret.expires_at)
    }

    async fn get_for_injection(
        &self,
        user_id: &str,
        name: &str,
        context: SecretAccessContext,
    ) -> Result<DecryptedSecret, SecretError> {
        SecretsStore::get_for_injection(self, user_id, name, context).await
    }

    async fn list_secret_names(&self, user_id: &str) -> Result<Vec<String>, SecretError> {
        self.list(user_id)
            .await
            .map(|secrets| secrets.into_iter().map(|secret| secret.name).collect())
    }

    async fn create_secret(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<(), SecretError> {
        self.create(user_id, params).await.map(|_| ())
    }

    async fn record_leak_detection_event(
        &self,
        user_id: &str,
        source: &str,
        action_taken: &str,
        content_hash: &str,
        redacted_preview: Option<&str>,
    ) -> Result<(), SecretError> {
        SecretsStore::record_leak_detection_event(
            self,
            user_id,
            source,
            action_taken,
            content_hash,
            redacted_preview,
        )
        .await
    }
}

/// A single leak match observed at a WASM host boundary.
#[derive(Debug, Clone)]
pub struct LeakScanMatch {
    pub pattern_name: String,
    pub action_taken: String,
    pub masked_preview: String,
}

/// Result returned by a leak scanner.
#[derive(Debug, Clone, Default)]
pub struct LeakScan {
    pub matches: Vec<LeakScanMatch>,
    pub should_block: bool,
    pub redacted_content: Option<String>,
}

/// Boundary scanner for data entering/leaving WASM host functions.
pub trait LeakScanner: Send + Sync {
    fn scan(&self, content: &str, exact_values: &[String]) -> LeakScan;
}

/// Default exact-value scanner used when the app does not provide a richer scanner.
#[derive(Debug, Default)]
pub struct ExactValueLeakScanner;

impl LeakScanner for ExactValueLeakScanner {
    fn scan(&self, content: &str, exact_values: &[String]) -> LeakScan {
        let mut scan = LeakScan::default();
        let mut redacted = content.to_string();
        for value in exact_values {
            if value.len() < 4 || !content.contains(value) {
                continue;
            }
            scan.should_block = true;
            scan.matches.push(LeakScanMatch {
                pattern_name: "exact_loaded_secret".to_string(),
                action_taken: "block".to_string(),
                masked_preview: mask_secret(value),
            });
            redacted = redacted.replace(value, "[REDACTED]");
        }
        if redacted != content {
            scan.redacted_content = Some(redacted);
        }
        scan
    }
}

fn mask_secret(value: &str) -> String {
    let char_count = value.chars().count();
    if char_count <= 8 {
        return "*".repeat(char_count);
    }
    let prefix: String = value.chars().take(4).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

/// Registration data passed from generic loaders into an app registry adapter.
pub struct WasmToolRegistration<'a, S: SecretResolver + ?Sized, I: HostToolInvoker + ?Sized> {
    pub name: &'a str,
    pub wasm_bytes: &'a [u8],
    pub runtime: &'a Arc<WasmToolRuntime>,
    pub capabilities: Capabilities,
    pub limits: Option<ResourceLimits>,
    pub description: Option<&'a str>,
    pub schema: Option<serde_json::Value>,
    pub secrets: Option<Arc<S>>,
    pub oauth_refresh: Option<OAuthRefreshConfig>,
    pub tool_invoker: Option<Arc<I>>,
}

/// Registry operations needed by the generic loader.
#[async_trait]
pub trait WasmToolRegistrar: Send + Sync {
    type SecretResolver: SecretResolver + ?Sized + 'static;
    type ToolInvoker: HostToolInvoker + ?Sized + 'static;
    type Error: std::fmt::Display + Send + Sync + 'static;

    async fn register_wasm(
        &self,
        reg: WasmToolRegistration<'_, Self::SecretResolver, Self::ToolInvoker>,
    ) -> Result<(), Self::Error>;

    async fn register_wasm_from_storage(
        &self,
        store: &dyn WasmToolStore,
        runtime: &Arc<WasmToolRuntime>,
        user_id: &str,
        name: &str,
        tool_invoker: Option<Arc<Self::ToolInvoker>>,
    ) -> Result<(), Self::Error>;
}

/// Registry removal needed by the watcher.
#[async_trait]
pub trait RegistryUnregister: Send + Sync {
    async fn unregister(&self, name: &str) -> Option<Arc<dyn Tool>>;
}

impl From<WasmStorageError> for String {
    fn from(error: WasmStorageError) -> Self {
        error.to_string()
    }
}

impl From<WasmError> for String {
    fn from(error: WasmError) -> Self {
        error.to_string()
    }
}
