//! Executable, host-mediated plugin contribution runtime.
//!
//! Memory and context providers use a deliberately small HTTP/JSON contract.
//! The host owns DNS/SSRF validation, timeouts, byte limits, credential
//! injection, selection, health, and shutdown. Provider output is returned as
//! untrusted prompt material and still passes through the normal prompt
//! sanitation layer.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, watch};

use crate::extensions::manifest::{
    ContextProviderContribution, ExtensionHttpAuth, HttpJsonProviderRuntime,
    MemoryProviderContribution, PluginManifest,
};
use crate::extensions::registry::PluginManifestRegistrationError;
use crate::identity::AccessContext;
use crate::secrets::{DecryptedSecret, SecretAccessContext, SecretsStore};
use crate::settings::ExtensionsSettings;

use super::manager::ExtensionManager;

const MAX_MANIFEST_DIRECTORY_ENTRIES: usize = 4_096;
const MAX_MANIFESTS_PER_DIRECTORY: usize = 256;
const MAX_MANIFEST_DIRS: usize = 64;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_ACTIVE_CONTEXT_PROVIDERS: usize = 8;
const MAX_PROVIDER_ITEMS: usize = 64;
const MAX_PROVIDER_ITEM_BYTES: usize = 64 * 1024;

fn valid_qualified_selection(value: &str) -> bool {
    let Some((manifest, contribution)) = value.split_once('/') else {
        return false;
    };
    !manifest.is_empty()
        && !contribution.is_empty()
        && value.len() <= 257
        && !contribution.contains('/')
        && manifest
            .bytes()
            .chain(contribution.bytes())
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionProviderKind {
    Memory,
    Context,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionProviderState {
    Registered,
    Active,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContributionProviderStatus {
    pub id: String,
    pub kind: ContributionProviderKind,
    pub state: ContributionProviderState,
    /// Stable categorical code only. Provider URLs, response bodies, paths,
    /// and secret names are intentionally never retained in status state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip)]
    generation: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContributionPromptContext {
    pub memory: Vec<ContributionContextItem>,
    pub context: Vec<ContributionContextItem>,
}

impl ContributionPromptContext {
    pub fn render(&self) -> Option<String> {
        let mut sections = Vec::new();
        if !self.memory.is_empty() {
            sections.push(render_items("Extension Memory Recall", &self.memory));
        }
        if !self.context.is_empty() {
            sections.push(render_items("Extension Context", &self.context));
        }
        (!sections.is_empty()).then(|| sections.join("\n\n"))
    }
}

fn render_items(title: &str, items: &[ContributionContextItem]) -> String {
    let mut rendered = format!("## {title}");
    for item in items {
        rendered.push_str("\n\n- ");
        rendered.push_str(&item.content);
        if let Some(reference) = &item.reference {
            rendered.push_str("\n  Reference: ");
            rendered.push_str(reference);
        }
    }
    rendered
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributionContextItem {
    pub provider_id: String,
    pub content: String,
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContributionScanReport {
    pub manifests_registered: Vec<String>,
    pub manifests_rejected: Vec<String>,
}

#[derive(Clone)]
struct ProviderRecord {
    manifest_id: String,
    qualified_id: String,
    runtime: HttpJsonProviderRuntime,
    kind: ProviderRecordKind,
    generation: u64,
}

#[derive(Clone)]
enum ProviderRecordKind {
    Memory(crate::extensions::manifest::MemoryProviderOperations),
    Context(crate::extensions::manifest::ContextProviderOperations),
}

impl ProviderRecord {
    fn kind(&self) -> ContributionProviderKind {
        match &self.kind {
            ProviderRecordKind::Memory(_) => ContributionProviderKind::Memory,
            ProviderRecordKind::Context(_) => ContributionProviderKind::Context,
        }
    }

    fn health_path(&self) -> &str {
        match &self.kind {
            ProviderRecordKind::Memory(operations) => &operations.health_path,
            ProviderRecordKind::Context(operations) => &operations.health_path,
        }
    }
}

#[derive(Clone, Copy)]
enum HttpMethod {
    Get,
    Post,
}

struct ContributionHttpRequest {
    url: String,
    method: HttpMethod,
    timeout: Duration,
    max_response_bytes: usize,
    body: Option<Vec<u8>>,
    bearer: Option<DecryptedSecret>,
}

#[async_trait]
trait ContributionHttpTransport: Send + Sync {
    async fn send(&self, request: ContributionHttpRequest) -> Result<Vec<u8>, InvocationFailure>;
}

struct SecureContributionHttpTransport;

#[async_trait]
impl ContributionHttpTransport for SecureContributionHttpTransport {
    async fn send(&self, request: ContributionHttpRequest) -> Result<Vec<u8>, InvocationFailure> {
        let options = thinclaw_tools_core::OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: false,
            allowlist: Vec::new(),
        };
        let guarded =
            thinclaw_tools_core::validate_outbound_url_pinned_async(&request.url, &options)
                .await
                .map_err(|_| InvocationFailure::new("outbound_policy_denied"))?;
        let host = guarded
            .url
            .host_str()
            .ok_or_else(|| InvocationFailure::new("outbound_policy_denied"))?
            .to_string();
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(request.timeout)
            .connect_timeout(request.timeout)
            .no_proxy();
        if !guarded.pinned_addrs.is_empty() {
            builder = builder.resolve_to_addrs(&host, &guarded.pinned_addrs);
        }
        let client = builder
            .build()
            .map_err(|_| InvocationFailure::new("http_client_unavailable"))?;
        let mut request_builder = match request.method {
            HttpMethod::Get => client.get(guarded.url),
            HttpMethod::Post => client.post(guarded.url),
        };
        if let Some(body) = request.body {
            request_builder = request_builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        if let Some(secret) = request.bearer {
            let mut value =
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", secret.expose()))
                    .map_err(|_| InvocationFailure::new("secret_invalid"))?;
            value.set_sensitive(true);
            request_builder = request_builder.header(reqwest::header::AUTHORIZATION, value);
        }
        let response = request_builder
            .send()
            .await
            .map_err(|_| InvocationFailure::new("http_request_failed"))?;
        if !response.status().is_success() {
            return Err(InvocationFailure::new("provider_rejected_request"));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| InvocationFailure::new("http_response_failed"))?;
            if bytes.len().saturating_add(chunk.len()) > request.max_response_bytes {
                return Err(InvocationFailure::new("response_too_large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[derive(Debug, Clone)]
struct InvocationFailure {
    code: &'static str,
}

impl InvocationFailure {
    const fn new(code: &'static str) -> Self {
        Self { code }
    }
}

pub(crate) struct ContributionRuntime {
    providers: RwLock<HashMap<String, ProviderRecord>>,
    statuses: RwLock<HashMap<String, ContributionProviderStatus>>,
    activation_lock: Mutex<()>,
    running: AtomicBool,
    shutdown_tx: watch::Sender<bool>,
    next_generation: AtomicU64,
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    user_id: String,
    transport: Arc<dyn ContributionHttpTransport>,
}

impl ContributionRuntime {
    pub(super) fn new(secrets: Arc<dyn SecretsStore + Send + Sync>, user_id: String) -> Self {
        Self::with_transport(secrets, user_id, Arc::new(SecureContributionHttpTransport))
    }

    fn with_transport(
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        user_id: String,
        transport: Arc<dyn ContributionHttpTransport>,
    ) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            providers: RwLock::new(HashMap::new()),
            statuses: RwLock::new(HashMap::new()),
            activation_lock: Mutex::new(()),
            running: AtomicBool::new(true),
            shutdown_tx,
            next_generation: AtomicU64::new(1),
            secrets,
            user_id,
            transport,
        }
    }

    pub(crate) async fn register_manifest(&self, manifest: &PluginManifest) {
        if !self.running.load(Ordering::Acquire) {
            return;
        }
        let mut providers = self.providers.write().await;
        let mut statuses = self.statuses.write().await;
        if !self.running.load(Ordering::Acquire) {
            return;
        }
        let replaced_ids: Vec<_> = providers
            .iter()
            .filter(|(_, record)| record.manifest_id == manifest.id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in replaced_ids {
            providers.remove(&id);
            statuses.remove(&id);
        }
        for provider in &manifest.contributions.memory_providers {
            let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
            if let Some(record) = memory_record(manifest, provider, generation) {
                register_record(&mut providers, &mut statuses, record);
            }
        }
        for provider in &manifest.contributions.context_providers {
            let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
            if let Some(record) = context_record(manifest, provider, generation) {
                register_record(&mut providers, &mut statuses, record);
            }
        }
    }

    pub(crate) async fn activate_selected(&self, settings: &ExtensionsSettings) {
        if !self.running.load(Ordering::Acquire) {
            return;
        }
        let _activation_guard = self.activation_lock.lock().await;
        if !self.running.load(Ordering::Acquire) {
            return;
        }
        let mut selected = Vec::new();
        if let Some(memory) = settings
            .active_memory_provider
            .as_deref()
            .filter(|memory| valid_qualified_selection(memory))
        {
            selected.push(memory.to_string());
        }
        selected.extend(
            settings
                .active_context_providers
                .iter()
                .filter(|id| valid_qualified_selection(id))
                .take(MAX_ACTIVE_CONTEXT_PROVIDERS)
                .cloned(),
        );
        let mut selected_set = HashSet::new();
        selected.retain(|id| selected_set.insert(id.clone()));
        for status in self.statuses.write().await.values_mut() {
            if !selected_set.contains(&status.id) {
                status.state = ContributionProviderState::Registered;
                status.error_code = None;
            }
        }
        futures::future::join_all(selected.iter().map(|id| self.ensure_active(id, true))).await;
    }

    async fn ensure_active(
        &self,
        id: &str,
        retry_unhealthy: bool,
    ) -> Result<ProviderRecord, InvocationFailure> {
        if !self.running.load(Ordering::Acquire) {
            return Err(InvocationFailure::new("runtime_stopped"));
        }
        let record = self
            .providers
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| InvocationFailure::new("provider_not_registered"))?;
        if let Some(status) = self.statuses.read().await.get(id)
            && status.generation == record.generation
        {
            match status.state {
                ContributionProviderState::Active => return Ok(record),
                ContributionProviderState::Unhealthy if !retry_unhealthy => {
                    return Err(InvocationFailure::new("provider_unhealthy"));
                }
                ContributionProviderState::Registered | ContributionProviderState::Unhealthy => {}
            }
        }
        match self.invoke_health(&record).await {
            Ok(()) => {
                self.record_status(&record, ContributionProviderState::Active, None)
                    .await;
                Ok(record)
            }
            Err(error) => {
                self.record_status(
                    &record,
                    ContributionProviderState::Unhealthy,
                    Some(error.code),
                )
                .await;
                Err(error)
            }
        }
    }

    async fn invoke_health(&self, record: &ProviderRecord) -> Result<(), InvocationFailure> {
        let response = self
            .send(record, HttpMethod::Get, record.health_path(), None)
            .await?;
        let health: HealthResponse = serde_json::from_slice(&response)
            .map_err(|_| InvocationFailure::new("invalid_health_response"))?;
        health
            .healthy
            .then_some(())
            .ok_or_else(|| InvocationFailure::new("provider_unhealthy"))
    }

    async fn recall(
        &self,
        id: &str,
        access: &AccessContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ContributionContextItem>, InvocationFailure> {
        let record = self.ensure_active(id, false).await?;
        let ProviderRecordKind::Memory(operations) = &record.kind else {
            return Err(InvocationFailure::new("provider_kind_mismatch"));
        };
        let body = encode_json_bounded(
            &MemoryRecallRequest {
                subject_id: access.provider_subject_id(),
                query,
                limit: limit.min(MAX_PROVIDER_ITEMS),
                conversation_kind: access.conversation_kind.as_str(),
                channel: &access.channel,
            },
            record.runtime.max_request_bytes,
        )?;
        let response = self
            .send(
                &record,
                HttpMethod::Post,
                &operations.recall_path,
                Some(body),
            )
            .await;
        self.decode_items(&record, response).await
    }

    async fn resolve_context(
        &self,
        id: &str,
        access: &AccessContext,
        query: &str,
    ) -> Result<Vec<ContributionContextItem>, InvocationFailure> {
        let record = self.ensure_active(id, false).await?;
        let ProviderRecordKind::Context(operations) = &record.kind else {
            return Err(InvocationFailure::new("provider_kind_mismatch"));
        };
        let body = encode_json_bounded(
            &ContextResolveRequest {
                subject_id: access.provider_subject_id(),
                query,
                conversation_kind: access.conversation_kind.as_str(),
                channel: &access.channel,
            },
            record.runtime.max_request_bytes,
        )?;
        let response = self
            .send(
                &record,
                HttpMethod::Post,
                &operations.resolve_path,
                Some(body),
            )
            .await;
        self.decode_items(&record, response).await
    }

    async fn store_memory(
        &self,
        id: &str,
        access: &AccessContext,
        payload: &serde_json::Value,
    ) -> Result<(), InvocationFailure> {
        let record = self.ensure_active(id, false).await?;
        let ProviderRecordKind::Memory(operations) = &record.kind else {
            return Err(InvocationFailure::new("provider_kind_mismatch"));
        };
        let body = encode_json_bounded(
            &MemoryStoreRequest {
                subject_id: access.provider_subject_id(),
                conversation_kind: access.conversation_kind.as_str(),
                payload,
            },
            record.runtime.max_request_bytes,
        )?;
        let result = self
            .send(
                &record,
                HttpMethod::Post,
                &operations.store_path,
                Some(body),
            )
            .await;
        if let Err(error) = result {
            self.record_status(
                &record,
                ContributionProviderState::Unhealthy,
                Some(error.code),
            )
            .await;
            tracing::debug!(
                provider = %record.qualified_id,
                error_code = error.code,
                "Extension memory write failed in isolation"
            );
            return Err(error);
        }
        self.record_status(&record, ContributionProviderState::Active, None)
            .await;
        Ok(())
    }

    async fn decode_items(
        &self,
        record: &ProviderRecord,
        response: Result<Vec<u8>, InvocationFailure>,
    ) -> Result<Vec<ContributionContextItem>, InvocationFailure> {
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                self.record_status(
                    record,
                    ContributionProviderState::Unhealthy,
                    Some(error.code),
                )
                .await;
                return Err(error);
            }
        };
        let response: ItemsResponse = match serde_json::from_slice(&response) {
            Ok(response) => response,
            Err(_) => {
                let error = InvocationFailure::new("invalid_provider_response");
                self.record_status(
                    record,
                    ContributionProviderState::Unhealthy,
                    Some(error.code),
                )
                .await;
                return Err(error);
            }
        };
        if response.items.len() > MAX_PROVIDER_ITEMS {
            let error = InvocationFailure::new("too_many_provider_items");
            self.record_status(
                record,
                ContributionProviderState::Unhealthy,
                Some(error.code),
            )
            .await;
            return Err(error);
        }
        let mut items = Vec::with_capacity(response.items.len());
        for item in response.items {
            if item.content.is_empty()
                || item.content.len() > MAX_PROVIDER_ITEM_BYTES
                || item.content.contains('\0')
                || item
                    .reference
                    .as_deref()
                    .is_some_and(|reference| reference.len() > 2_048 || reference.contains('\0'))
            {
                let error = InvocationFailure::new("invalid_provider_item");
                self.record_status(
                    record,
                    ContributionProviderState::Unhealthy,
                    Some(error.code),
                )
                .await;
                return Err(error);
            }
            items.push(ContributionContextItem {
                provider_id: record.qualified_id.clone(),
                content: item.content,
                reference: item.reference,
            });
        }
        self.record_status(record, ContributionProviderState::Active, None)
            .await;
        Ok(items)
    }

    async fn send(
        &self,
        record: &ProviderRecord,
        method: HttpMethod,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, InvocationFailure> {
        if !self.running.load(Ordering::Acquire) {
            return Err(InvocationFailure::new("runtime_stopped"));
        }
        if body.as_ref().is_some_and(|body| {
            body.len() > usize::try_from(record.runtime.max_request_bytes).unwrap_or(usize::MAX)
        }) {
            return Err(InvocationFailure::new("request_too_large"));
        }
        let url = format!("{}{}", record.runtime.base_url.trim_end_matches('/'), path);
        let bearer = match &record.runtime.auth {
            ExtensionHttpAuth::None => None,
            ExtensionHttpAuth::BearerSecret { binding } => {
                let parsed = reqwest::Url::parse(&url)
                    .map_err(|_| InvocationFailure::new("outbound_policy_denied"))?;
                let host = parsed
                    .host_str()
                    .ok_or_else(|| InvocationFailure::new("outbound_policy_denied"))?;
                Some(
                    self.while_running(async {
                        self.secrets
                            .get_for_injection(
                                &self.user_id,
                                binding,
                                SecretAccessContext::new(
                                    "extensions.contribution_runtime",
                                    "provider_http_bearer",
                                )
                                .target(host, path)
                                .auth_source(record.qualified_id.clone()),
                            )
                            .await
                            .map_err(|_| InvocationFailure::new("secret_unavailable"))
                    })
                    .await?,
                )
            }
        };
        self.while_running(self.transport.send(ContributionHttpRequest {
            url,
            method,
            timeout: Duration::from_millis(record.runtime.timeout_ms),
            max_response_bytes:
                usize::try_from(record.runtime.max_response_bytes).unwrap_or(usize::MAX),
            body,
            bearer,
        }))
        .await
    }

    async fn while_running<T, F>(&self, future: F) -> Result<T, InvocationFailure>
    where
        F: Future<Output = Result<T, InvocationFailure>>,
    {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        if *shutdown_rx.borrow() || !self.running.load(Ordering::Acquire) {
            return Err(InvocationFailure::new("runtime_stopped"));
        }
        tokio::select! {
            response = future => response,
            _ = shutdown_rx.changed() => Err(InvocationFailure::new("runtime_stopped")),
        }
    }

    async fn record_status(
        &self,
        record: &ProviderRecord,
        state: ContributionProviderState,
        error_code: Option<&str>,
    ) {
        let providers = self.providers.read().await;
        if !providers
            .get(&record.qualified_id)
            .is_some_and(|current| current.generation == record.generation)
        {
            return;
        }
        let mut statuses = self.statuses.write().await;
        if !self.running.load(Ordering::Acquire) {
            return;
        }
        statuses.insert(
            record.qualified_id.clone(),
            ContributionProviderStatus {
                id: record.qualified_id.clone(),
                kind: record.kind(),
                state,
                error_code: error_code.map(str::to_string),
                generation: record.generation,
            },
        );
    }

    async fn statuses(&self) -> Vec<ContributionProviderStatus> {
        let mut statuses: Vec<_> = self.statuses.read().await.values().cloned().collect();
        statuses.sort_by(|left, right| left.id.cmp(&right.id));
        statuses
    }

    async fn shutdown(&self) {
        self.running.store(false, Ordering::Release);
        self.shutdown_tx.send_replace(true);
        let _activation_guard = self.activation_lock.lock().await;
        for status in self.statuses.write().await.values_mut() {
            status.state = ContributionProviderState::Registered;
            status.error_code = None;
        }
    }
}

fn memory_record(
    manifest: &PluginManifest,
    provider: &MemoryProviderContribution,
    generation: u64,
) -> Option<ProviderRecord> {
    Some(ProviderRecord {
        manifest_id: manifest.id.clone(),
        qualified_id: qualified_id(manifest, &provider.id),
        runtime: provider.runtime.clone()?,
        kind: ProviderRecordKind::Memory(provider.operations.clone()?),
        generation,
    })
}

fn context_record(
    manifest: &PluginManifest,
    provider: &ContextProviderContribution,
    generation: u64,
) -> Option<ProviderRecord> {
    Some(ProviderRecord {
        manifest_id: manifest.id.clone(),
        qualified_id: qualified_id(manifest, &provider.id),
        runtime: provider.runtime.clone()?,
        kind: ProviderRecordKind::Context(provider.operations.clone()?),
        generation,
    })
}

fn qualified_id(manifest: &PluginManifest, contribution_id: &str) -> String {
    format!("{}/{}", manifest.id, contribution_id)
}

fn register_record(
    providers: &mut HashMap<String, ProviderRecord>,
    statuses: &mut HashMap<String, ContributionProviderStatus>,
    record: ProviderRecord,
) {
    statuses.insert(
        record.qualified_id.clone(),
        ContributionProviderStatus {
            id: record.qualified_id.clone(),
            kind: record.kind(),
            state: ContributionProviderState::Registered,
            error_code: None,
            generation: record.generation,
        },
    );
    providers.insert(record.qualified_id.clone(), record);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryRecallRequest<'a> {
    subject_id: String,
    query: &'a str,
    limit: usize,
    conversation_kind: &'a str,
    channel: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextResolveRequest<'a> {
    subject_id: String,
    query: &'a str,
    conversation_kind: &'a str,
    channel: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryStoreRequest<'a> {
    subject_id: String,
    conversation_kind: &'a str,
    payload: &'a serde_json::Value,
}

#[derive(Deserialize)]
struct HealthResponse {
    healthy: bool,
}

#[derive(Deserialize)]
struct ItemsResponse {
    items: Vec<ProviderItem>,
}

#[derive(Deserialize)]
struct ProviderItem {
    content: String,
    #[serde(default)]
    reference: Option<String>,
}

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl std::io::Write for BoundedJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(buffer.len()) > self.limit {
            self.exceeded = true;
            return Err(std::io::Error::other(
                "bounded JSON request exceeded its limit",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encode_json_bounded(value: &impl Serialize, limit: u64) -> Result<Vec<u8>, InvocationFailure> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut writer = BoundedJsonWriter {
        bytes: Vec::with_capacity(limit.min(16 * 1024)),
        limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(writer.bytes),
        Err(_) if writer.exceeded => Err(InvocationFailure::new("request_too_large")),
        Err(_) => Err(InvocationFailure::new("request_encoding_failed")),
    }
}

impl ExtensionManager {
    /// Scan the configured broad-manifest directories. Unlike the native
    /// scanner, this path never loads code and does not require native opt-in.
    pub async fn register_contribution_manifests_from_config(&self) -> ContributionScanReport {
        let settings = self.current_extensions_settings().await;
        let mut report = ContributionScanReport::default();
        for directory in settings
            .contribution_manifest_dirs
            .iter()
            .take(MAX_MANIFEST_DIRS)
        {
            if directory.is_empty() || directory.len() > 4_096 || directory.contains('\0') {
                report
                    .manifests_rejected
                    .push("invalid_directory".to_string());
                continue;
            }
            let expanded = crate::platform::expand_home_dir(directory);
            self.scan_contribution_manifest_directory(Path::new(&expanded), &settings, &mut report)
                .await;
        }
        self.contribution_runtime.activate_selected(&settings).await;
        report
    }

    async fn scan_contribution_manifest_directory(
        &self,
        directory: &Path,
        settings: &ExtensionsSettings,
        report: &mut ContributionScanReport,
    ) {
        if !tokio::fs::symlink_metadata(directory)
            .await
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            report
                .manifests_rejected
                .push("unreadable_directory".to_string());
            return;
        }
        let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
            report
                .manifests_rejected
                .push("unreadable_directory".to_string());
            return;
        };
        let mut entry_count = 0usize;
        let mut manifest_count = 0usize;
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(_) => {
                    report
                        .manifests_rejected
                        .push("directory_read_failed".to_string());
                    break;
                }
            };
            entry_count = entry_count.saturating_add(1);
            if entry_count > MAX_MANIFEST_DIRECTORY_ENTRIES {
                report
                    .manifests_rejected
                    .push("directory_entry_limit".to_string());
                break;
            }
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            if !tokio::fs::symlink_metadata(&path)
                .await
                .is_ok_and(|metadata| {
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && metadata.len() <= MAX_MANIFEST_BYTES as u64
                })
            {
                report
                    .manifests_rejected
                    .push("invalid_manifest_file".to_string());
                continue;
            }
            manifest_count = manifest_count.saturating_add(1);
            if manifest_count > MAX_MANIFESTS_PER_DIRECTORY {
                report
                    .manifests_rejected
                    .push("manifest_count_limit".to_string());
                break;
            }
            let bytes = match thinclaw_platform::read_regular_file_bounded_single_link(
                &path,
                MAX_MANIFEST_BYTES as u64,
            ) {
                Ok(bytes) => bytes,
                Err(_) => {
                    report
                        .manifests_rejected
                        .push("manifest_read_failed".to_string());
                    continue;
                }
            };
            let manifest: PluginManifest = match serde_json::from_slice(&bytes) {
                Ok(manifest) => manifest,
                Err(_) => {
                    report
                        .manifests_rejected
                        .push("manifest_parse_failed".to_string());
                    continue;
                }
            };
            if !has_host_contributions(&manifest) {
                continue;
            }
            let manifest_label = safe_manifest_label(&manifest.id);
            if manifest_label == manifest.id
                && report
                    .manifests_registered
                    .iter()
                    .any(|registered| registered == &manifest.id)
            {
                tracing::warn!(
                    manifest = %manifest_label,
                    "Duplicate extension contribution manifest id was rejected"
                );
                report.manifests_rejected.push(manifest_label);
                continue;
            }
            match self
                .register_contribution_manifest(&manifest, settings)
                .await
            {
                Ok(()) => report.manifests_registered.push(manifest.id),
                Err(_) => {
                    tracing::warn!(
                        manifest = %manifest_label,
                        "Extension contribution manifest was rejected"
                    );
                    report.manifests_rejected.push(manifest_label);
                }
            }
        }
    }

    pub(crate) async fn register_contribution_manifest(
        &self,
        manifest: &PluginManifest,
        settings: &ExtensionsSettings,
    ) -> Result<(), PluginManifestRegistrationError> {
        self.registry
            .register_plugin_manifest_contributions(manifest, settings)
            .await?;
        self.contribution_runtime.register_manifest(manifest).await;
        Ok(())
    }

    /// Resolve selected memory/context contributions for a real prompt turn.
    /// Each provider fails independently; a broken extension never blocks the
    /// built-in prompt path or another extension.
    pub async fn contributed_prompt_context(
        &self,
        access: &AccessContext,
        query: &str,
        limit: usize,
    ) -> ContributionPromptContext {
        let settings = self.current_extensions_settings().await;
        self.contribution_runtime.activate_selected(&settings).await;
        let memory_future = async {
            let Some(id) = settings.active_memory_provider.as_deref() else {
                return Vec::new();
            };
            if !valid_qualified_selection(id) {
                return Vec::new();
            }
            self.contribution_runtime
                .recall(id, access, query, limit)
                .await
                .unwrap_or_else(|error| {
                    tracing::debug!(
                        provider = id,
                        error_code = error.code,
                        "Extension memory recall failed in isolation"
                    );
                    Vec::new()
                })
        };
        let mut seen_context = HashSet::new();
        let context_futures = settings
            .active_context_providers
            .iter()
            .filter(|id| valid_qualified_selection(id))
            .filter(|id| seen_context.insert((*id).clone()))
            .take(MAX_ACTIVE_CONTEXT_PROVIDERS)
            .map(|id| async move {
                self.contribution_runtime
                    .resolve_context(id, access, query)
                    .await
                    .unwrap_or_else(|error| {
                        tracing::debug!(
                            provider = id,
                            error_code = error.code,
                            "Extension context resolution failed in isolation"
                        );
                        Vec::new()
                    })
            });
        let (memory, context_results) =
            tokio::join!(memory_future, futures::future::join_all(context_futures));
        ContributionPromptContext {
            memory,
            context: context_results.into_iter().flatten().collect(),
        }
    }

    /// Mirror a completed turn to the selected contributed memory provider.
    /// Writes are best-effort and isolated from durable local run logging.
    pub async fn sync_contributed_memory(
        &self,
        access: &AccessContext,
        payload: &serde_json::Value,
    ) {
        let settings = self.current_extensions_settings().await;
        let Some(id) = settings.active_memory_provider.as_deref() else {
            return;
        };
        if !valid_qualified_selection(id) {
            return;
        }
        let _ = self
            .contribution_runtime
            .store_memory(id, access, payload)
            .await;
    }

    pub async fn contribution_provider_statuses(&self) -> Vec<ContributionProviderStatus> {
        self.contribution_runtime.statuses().await
    }

    pub async fn stop_contribution_runtime(&self) {
        self.contribution_runtime.shutdown().await;
    }
}

pub(crate) fn safe_manifest_label(id: &str) -> String {
    if !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        id.to_string()
    } else {
        "invalid_manifest_id".to_string()
    }
}

fn has_host_contributions(manifest: &PluginManifest) -> bool {
    !manifest.contributions.tools.is_empty()
        || !manifest.contributions.channels.is_empty()
        || !manifest.contributions.memory_providers.is_empty()
        || !manifest.contributions.context_providers.is_empty()
        || !manifest.contributions.auth_providers.is_empty()
        || !manifest.contributions.llm_providers.is_empty()
        || !manifest.contributions.http_routes.is_empty()
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};

    use secrecy::SecretString;
    use uuid::Uuid;

    use super::*;
    use crate::identity::ConversationKind;
    use crate::secrets::{InMemorySecretsStore, SecretsCrypto};

    #[derive(Default)]
    struct FakeTransport {
        responses: Mutex<HashMap<String, VecDeque<Result<Vec<u8>, &'static str>>>>,
        requests: Mutex<Vec<CapturedRequest>>,
    }

    struct CapturedRequest {
        path: String,
        body: Option<serde_json::Value>,
        had_bearer: bool,
    }

    #[derive(Default)]
    struct BlockingTransport {
        request_started: tokio::sync::Notify,
    }

    #[async_trait]
    impl ContributionHttpTransport for BlockingTransport {
        async fn send(
            &self,
            request: ContributionHttpRequest,
        ) -> Result<Vec<u8>, InvocationFailure> {
            if matches!(request.method, HttpMethod::Get) {
                return Ok(br#"{"healthy":true}"#.to_vec());
            }
            self.request_started.notify_one();
            std::future::pending().await
        }
    }

    impl FakeTransport {
        async fn respond_json(&self, path: &str, value: serde_json::Value) {
            self.responses
                .lock()
                .await
                .entry(path.to_string())
                .or_default()
                .push_back(Ok(
                    serde_json::to_vec(&value).expect("fixture response serializes")
                ));
        }

        async fn fail(&self, path: &str, code: &'static str) {
            self.responses
                .lock()
                .await
                .entry(path.to_string())
                .or_default()
                .push_back(Err(code));
        }
    }

    #[async_trait]
    impl ContributionHttpTransport for FakeTransport {
        async fn send(
            &self,
            request: ContributionHttpRequest,
        ) -> Result<Vec<u8>, InvocationFailure> {
            let path = reqwest::Url::parse(&request.url)
                .expect("validated fixture URL")
                .path()
                .to_string();
            let body = request
                .body
                .as_deref()
                .map(serde_json::from_slice)
                .transpose()
                .map_err(|_| InvocationFailure::new("invalid_test_request"))?;
            self.requests.lock().await.push(CapturedRequest {
                path: path.clone(),
                body,
                had_bearer: request.bearer.is_some(),
            });
            let response = self
                .responses
                .lock()
                .await
                .get_mut(&path)
                .and_then(VecDeque::pop_front)
                .ok_or_else(|| InvocationFailure::new("missing_test_response"))?
                .map_err(InvocationFailure::new)?;
            if response.len() > request.max_response_bytes {
                return Err(InvocationFailure::new("response_too_large"));
            }
            Ok(response)
        }
    }

    fn fixture_manifest() -> PluginManifest {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/extensions/executable-provider.json"
        ))
        .expect("fixture manifest parses")
    }

    fn access() -> AccessContext {
        AccessContext {
            principal_id: "principal-private".to_string(),
            actor_id: "actor-private".to_string(),
            conversation_scope_id: Uuid::new_v4(),
            conversation_kind: ConversationKind::Direct,
            channel: "test".to_string(),
        }
    }

    fn unsigned_settings() -> ExtensionsSettings {
        ExtensionsSettings {
            require_plugin_signatures: false,
            active_memory_provider: Some("fixture.providers/fixture.providers.memory".to_string()),
            active_context_providers: vec![
                "fixture.providers/fixture.providers.context".to_string(),
            ],
            ..ExtensionsSettings::default()
        }
    }

    fn runtime_with_fake(
        transport: Arc<FakeTransport>,
    ) -> (ContributionRuntime, Arc<InMemorySecretsStore>) {
        let crypto = Arc::new(
            SecretsCrypto::new(SecretString::from("11".repeat(32)))
                .expect("test crypto initializes"),
        );
        let secrets = Arc::new(InMemorySecretsStore::new(crypto));
        let runtime =
            ContributionRuntime::with_transport(secrets.clone(), "default".to_string(), transport);
        (runtime, secrets)
    }

    #[tokio::test]
    async fn fixture_activates_invokes_writes_and_isolates_provider_failure() {
        let manifest = fixture_manifest();
        let settings = unsigned_settings();
        let validation = crate::extensions::validate_plugin_manifest(&manifest, &settings);
        assert!(validation.valid, "{:?}", validation.errors);
        let registry = crate::extensions::registry::ExtensionRegistry::new();
        let registration = registry
            .register_plugin_manifest_contributions(&manifest, &settings)
            .await
            .expect("fixture crosses the production registry policy boundary");
        assert_eq!(
            registration.memory_providers_registered,
            vec!["fixture.providers.memory"]
        );
        assert_eq!(
            registration.context_providers_registered,
            vec!["fixture.providers.context"]
        );

        let transport = Arc::new(FakeTransport::default());
        transport
            .respond_json("/memory/health", serde_json::json!({ "healthy": true }))
            .await;
        transport
            .respond_json("/context/health", serde_json::json!({ "healthy": true }))
            .await;
        transport
            .respond_json(
                "/memory/recall",
                serde_json::json!({
                    "items": [{ "content": "remembered safely", "reference": "memory:1" }]
                }),
            )
            .await;
        transport
            .fail("/context/resolve", "provider_rejected_request")
            .await;
        transport
            .respond_json("/memory/store", serde_json::json!({ "accepted": true }))
            .await;

        let (runtime, _) = runtime_with_fake(transport.clone());
        runtime.register_manifest(&manifest).await;
        runtime.activate_selected(&settings).await;

        let access = access();
        let memory = runtime
            .recall(
                settings.active_memory_provider.as_deref().unwrap(),
                &access,
                "what matters?",
                6,
            )
            .await
            .expect("memory provider stays live");
        let context = runtime
            .resolve_context(
                &settings.active_context_providers[0],
                &access,
                "what matters?",
            )
            .await;
        assert_eq!(memory[0].content, "remembered safely");
        assert_eq!(
            context.expect_err("context fixture fails").code,
            "provider_rejected_request"
        );
        runtime
            .store_memory(
                settings.active_memory_provider.as_deref().unwrap(),
                &access,
                &serde_json::json!({ "turn": "completed" }),
            )
            .await
            .expect("memory write remains live after context failure");

        let statuses = runtime.statuses().await;
        assert_eq!(statuses.len(), 2);
        assert!(statuses.iter().any(|status| {
            status.kind == ContributionProviderKind::Memory
                && status.state == ContributionProviderState::Active
        }));
        assert!(statuses.iter().any(|status| {
            status.kind == ContributionProviderKind::Context
                && status.state == ContributionProviderState::Unhealthy
                && status.error_code.as_deref() == Some("provider_rejected_request")
        }));

        let requests = transport.requests.lock().await;
        let recall = requests
            .iter()
            .find(|request| request.path == "/memory/recall")
            .expect("recall request captured");
        let encoded = recall.body.as_ref().unwrap().to_string();
        assert!(encoded.contains("thinclaw-v1-"));
        assert!(!encoded.contains("principal-private"));
        assert!(!encoded.contains("actor-private"));
        drop(requests);

        runtime.shutdown().await;
        assert!(runtime.statuses().await.iter().all(|status| {
            status.state == ContributionProviderState::Registered && status.error_code.is_none()
        }));
    }

    #[tokio::test]
    async fn bearer_secret_is_resolved_only_at_the_host_boundary() {
        let mut manifest = fixture_manifest();
        let secret_name =
            "extension.fixture.providers.fixture.providers.memory.api_token".to_string();
        manifest
            .permissions
            .push(crate::extensions::manifest::EXTENSION_SECRETS_PERMISSION.to_string());
        manifest.contributions.memory_providers[0]
            .runtime
            .as_mut()
            .unwrap()
            .auth = ExtensionHttpAuth::BearerSecret {
            binding: secret_name.clone(),
        };
        let settings = unsigned_settings();
        let validation = crate::extensions::validate_plugin_manifest(&manifest, &settings);
        assert!(validation.valid, "{:?}", validation.errors);

        let transport = Arc::new(FakeTransport::default());
        transport
            .respond_json("/memory/health", serde_json::json!({ "healthy": true }))
            .await;
        transport
            .respond_json("/context/health", serde_json::json!({ "healthy": true }))
            .await;
        let (runtime, secrets) = runtime_with_fake(transport.clone());
        secrets
            .create(
                "default",
                crate::secrets::CreateSecretParams::new(&secret_name, "fixture-token"),
            )
            .await
            .expect("fixture secret stored encrypted");
        runtime.register_manifest(&manifest).await;
        runtime.activate_selected(&settings).await;

        let requests = transport.requests.lock().await;
        assert!(
            requests
                .iter()
                .any(|request| request.path == "/memory/health" && request.had_bearer)
        );
        drop(requests);
        let audit = secrets.access_audit_log().await;
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].context.caller, "extensions.contribution_runtime");
        assert_eq!(
            audit[0].context.target_host.as_deref(),
            Some("providers.example.com")
        );
        assert_eq!(
            audit[0].context.auth_source.as_deref(),
            Some("fixture.providers/fixture.providers.memory")
        );
    }

    #[tokio::test]
    async fn stale_inflight_generation_cannot_reactivate_replaced_provider() {
        let manifest = fixture_manifest();
        let transport = Arc::new(FakeTransport::default());
        let (runtime, _) = runtime_with_fake(transport);
        runtime.register_manifest(&manifest).await;
        let id = "fixture.providers/fixture.providers.memory";
        let old = runtime.providers.read().await.get(id).unwrap().clone();

        runtime.register_manifest(&manifest).await;
        let replacement_generation = runtime.providers.read().await.get(id).unwrap().generation;
        assert_ne!(old.generation, replacement_generation);

        runtime
            .record_status(&old, ContributionProviderState::Active, None)
            .await;
        let status = runtime.statuses.read().await.get(id).unwrap().clone();
        assert_eq!(status.state, ContributionProviderState::Registered);
        assert_eq!(status.generation, replacement_generation);
    }

    #[tokio::test]
    async fn manifest_replacement_removes_deleted_provider_capabilities() {
        let mut manifest = fixture_manifest();
        let transport = Arc::new(FakeTransport::default());
        let (runtime, _) = runtime_with_fake(transport);
        runtime.register_manifest(&manifest).await;
        assert_eq!(runtime.providers.read().await.len(), 2);

        manifest.contributions.context_providers.clear();
        runtime.register_manifest(&manifest).await;

        let providers = runtime.providers.read().await;
        assert_eq!(providers.len(), 1);
        assert!(providers.contains_key("fixture.providers/fixture.providers.memory"));
        assert!(!providers.contains_key("fixture.providers/fixture.providers.context"));
        drop(providers);
        assert_eq!(runtime.statuses().await.len(), 1);
    }

    #[tokio::test]
    async fn failed_activation_is_not_immediately_health_checked_twice() {
        let manifest = fixture_manifest();
        let mut settings = unsigned_settings();
        settings.active_context_providers.clear();
        let transport = Arc::new(FakeTransport::default());
        transport
            .fail("/memory/health", "http_request_failed")
            .await;
        let (runtime, _) = runtime_with_fake(transport.clone());
        runtime.register_manifest(&manifest).await;
        runtime.activate_selected(&settings).await;

        let error = runtime
            .recall(
                settings.active_memory_provider.as_deref().unwrap(),
                &access(),
                "do not retry yet",
                6,
            )
            .await
            .expect_err("unhealthy provider is skipped for this activation cycle");
        assert_eq!(error.code, "provider_unhealthy");
        let requests = transport.requests.lock().await;
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/memory/health")
                .count(),
            1
        );
        assert!(
            requests
                .iter()
                .all(|request| request.path != "/memory/recall")
        );
    }

    #[tokio::test]
    async fn request_body_limit_fails_before_transport_invocation() {
        let mut manifest = fixture_manifest();
        manifest.contributions.memory_providers[0]
            .runtime
            .as_mut()
            .unwrap()
            .max_request_bytes = 128;
        let transport = Arc::new(FakeTransport::default());
        transport
            .respond_json("/memory/health", serde_json::json!({ "healthy": true }))
            .await;
        let (runtime, _) = runtime_with_fake(transport.clone());
        runtime.register_manifest(&manifest).await;
        let error = runtime
            .recall(
                "fixture.providers/fixture.providers.memory",
                &access(),
                &"x".repeat(512),
                6,
            )
            .await
            .expect_err("oversized request must fail closed");
        assert_eq!(error.code, "request_too_large");
        assert!(
            transport
                .requests
                .lock()
                .await
                .iter()
                .all(|request| request.path != "/memory/recall")
        );
    }

    #[tokio::test]
    async fn shutdown_cancels_inflight_invocation_and_prevents_stale_reactivation() {
        let manifest = fixture_manifest();
        let transport = Arc::new(BlockingTransport::default());
        let crypto = Arc::new(
            SecretsCrypto::new(SecretString::from("22".repeat(32)))
                .expect("test crypto initializes"),
        );
        let secrets = Arc::new(InMemorySecretsStore::new(crypto));
        let runtime = Arc::new(ContributionRuntime::with_transport(
            secrets,
            "default".to_string(),
            transport.clone(),
        ));
        runtime.register_manifest(&manifest).await;
        let mut settings = unsigned_settings();
        settings.active_context_providers.clear();
        runtime.activate_selected(&settings).await;

        let invocation_runtime = Arc::clone(&runtime);
        let provider_id = settings.active_memory_provider.unwrap();
        let invocation = tokio::spawn(async move {
            invocation_runtime
                .recall(&provider_id, &access(), "shutdown race", 6)
                .await
        });
        transport.request_started.notified().await;
        runtime.shutdown().await;

        let error = tokio::time::timeout(Duration::from_secs(1), invocation)
            .await
            .expect("shutdown cancels transport promptly")
            .expect("invocation task does not panic")
            .expect_err("stopped runtime cannot complete an invocation");
        assert_eq!(error.code, "runtime_stopped");
        assert!(runtime.statuses().await.iter().all(|status| {
            status.state == ContributionProviderState::Registered && status.error_code.is_none()
        }));
    }
}
