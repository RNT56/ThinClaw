//! Multi-provider LLM failover.
//!
//! Wraps multiple LlmProvider instances and tries each in sequence
//! until one succeeds. Transparent to callers --- same LlmProvider trait.
//!
//! Providers that fail repeatedly are temporarily placed in cooldown
//! so subsequent requests skip them, reducing latency when a provider
//! is known to be down. Cooldown state is lock-free (atomics only).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;

use thinclaw_llm_core::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamSupport,
    TokenCaptureSupport, ToolCompletionRequest, ToolCompletionResponse,
};
use thinclaw_types::error::LlmError;

use crate::retry::is_retryable;

/// Configuration for per-provider cooldown behavior.
///
/// When a provider accumulates `failure_threshold` consecutive retryable
/// failures, it enters cooldown for `cooldown_duration`. During cooldown
/// the provider is skipped (unless *all* providers are in cooldown, in
/// which case the oldest-cooled one is tried).
#[derive(Debug, Clone)]
pub struct CooldownConfig {
    /// How long a provider stays in cooldown after exceeding the threshold.
    pub cooldown_duration: Duration,
    /// Number of consecutive retryable failures before cooldown activates.
    pub failure_threshold: u32,
}

impl Default for CooldownConfig {
    fn default() -> Self {
        Self {
            cooldown_duration: Duration::from_secs(300),
            failure_threshold: 3,
        }
    }
}

/// Per-provider cooldown state, entirely lock-free.
///
/// All atomic operations use `Relaxed` ordering — consistent with the
/// existing `last_used` field. Stale reads are harmless: the worst case
/// is one extra attempt against a provider that just entered cooldown.
struct ProviderCooldown {
    /// Consecutive retryable failures. Reset to 0 on success.
    failure_count: AtomicU32,
    /// Nanoseconds since `epoch` when cooldown was activated.
    /// 0 means the provider is NOT in cooldown.
    cooldown_activated_nanos: AtomicU64,
}

impl ProviderCooldown {
    fn new() -> Self {
        Self {
            failure_count: AtomicU32::new(0),
            cooldown_activated_nanos: AtomicU64::new(0),
        }
    }

    /// Check whether the provider is currently in cooldown.
    fn is_in_cooldown(&self, now_nanos: u64, cooldown_nanos: u64) -> bool {
        let activated = self.cooldown_activated_nanos.load(Ordering::Relaxed);
        activated != 0 && now_nanos.saturating_sub(activated) < cooldown_nanos
    }

    /// Record a retryable failure. Returns `true` if the threshold was
    /// just reached (caller should activate cooldown).
    fn record_failure(&self, threshold: u32) -> bool {
        let prev = self.failure_count.fetch_add(1, Ordering::Relaxed);
        prev + 1 >= threshold
    }

    /// Activate cooldown at the given timestamp.
    fn activate_cooldown(&self, now_nanos: u64) {
        // Ensure 0 remains a safe "not in cooldown" sentinel.
        self.cooldown_activated_nanos
            .store(now_nanos.max(1), Ordering::Relaxed);
    }

    /// Reset failure count and clear cooldown (called on success).
    fn reset(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        self.cooldown_activated_nanos.store(0, Ordering::Relaxed);
    }
}

/// Lease-selection strategy for concurrent provider usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseSelectionStrategy {
    FillFirst,
    RoundRobin,
    LeastUsed,
    Random,
}

/// Configuration for provider lease tracking.
#[derive(Debug, Clone)]
pub struct LeaseConfig {
    /// Maximum number of concurrent requests that may be leased to a single
    /// provider before failover prefers another available provider.
    pub max_concurrent: usize,
    /// Strategy used to order candidate providers when several are available.
    pub selection_strategy: LeaseSelectionStrategy,
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            selection_strategy: LeaseSelectionStrategy::FillFirst,
        }
    }
}

impl LeaseConfig {
    fn normalized(mut self) -> Self {
        self.max_concurrent = self.max_concurrent.max(1);
        self
    }
}

/// Process-local health snapshot for the credential lease pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialPoolHealthSnapshot {
    pub provider: String,
    pub entry_count: usize,
    pub max_concurrency: usize,
    pub active_lease_count: usize,
    pub selection_strategy: LeaseSelectionStrategy,
    pub oauth_sync_enabled: bool,
    pub last_sync_status: Option<String>,
}

/// Concrete provider entry participating in failover leasing.
///
/// Multiple entries may point at the same upstream provider family/model but
/// carry different credentials. The failover runtime leases these entries
/// independently so parallel requests can spread across credentials instead of
/// saturating a single shared provider bucket.
#[derive(Clone)]
pub struct ProviderLeaseEntry {
    pub provider: Arc<dyn LlmProvider>,
    pub lease_key: String,
}

impl ProviderLeaseEntry {
    pub fn new(provider: Arc<dyn LlmProvider>, lease_key: impl Into<String>) -> Self {
        Self {
            provider,
            lease_key: lease_key.into(),
        }
    }
}

struct LeaseTracker {
    active: Vec<Arc<AtomicUsize>>,
    served: Vec<AtomicUsize>,
    round_robin_cursor: AtomicUsize,
    config: LeaseConfig,
}

fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

impl LeaseTracker {
    fn new(provider_count: usize, config: LeaseConfig) -> Self {
        let config = config.normalized();
        Self {
            active: (0..provider_count)
                .map(|_| Arc::new(AtomicUsize::new(0)))
                .collect(),
            served: (0..provider_count).map(|_| AtomicUsize::new(0)).collect(),
            round_robin_cursor: AtomicUsize::new(0),
            config,
        }
    }

    fn order_candidates(&self, candidates: &[usize]) -> Vec<usize> {
        let mut ordered = candidates.to_vec();
        match self.config.selection_strategy {
            LeaseSelectionStrategy::FillFirst => ordered,
            LeaseSelectionStrategy::RoundRobin => {
                if !ordered.is_empty() {
                    let start = self.round_robin_cursor.fetch_add(1, Ordering::Relaxed);
                    let len = ordered.len();
                    ordered.rotate_left(start % len);
                }
                ordered
            }
            LeaseSelectionStrategy::LeastUsed => {
                ordered.sort_by_key(|&idx| {
                    (
                        self.active[idx].load(Ordering::Relaxed),
                        self.served[idx].load(Ordering::Relaxed),
                        idx,
                    )
                });
                ordered
            }
            LeaseSelectionStrategy::Random => {
                if ordered.len() <= 1 {
                    return ordered;
                }
                let mut seed = mix64(
                    self.round_robin_cursor
                        .fetch_add(1, Ordering::Relaxed)
                        .wrapping_add(1) as u64,
                );
                for i in (1..ordered.len()).rev() {
                    seed = mix64(seed.wrapping_add(i as u64));
                    let j = (seed as usize) % (i + 1);
                    ordered.swap(i, j);
                }
                ordered
            }
        }
    }

    fn try_acquire(&self, provider_idx: usize) -> Option<ProviderLease> {
        loop {
            let current = self.active[provider_idx].load(Ordering::Relaxed);
            if current >= self.config.max_concurrent {
                return None;
            }
            if self.active[provider_idx]
                .compare_exchange(current, current + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                self.served[provider_idx].fetch_add(1, Ordering::Relaxed);
                return Some(ProviderLease {
                    active: Arc::clone(&self.active[provider_idx]),
                });
            }
        }
    }
}

struct ProviderLease {
    active: Arc<AtomicUsize>,
}

impl Drop for ProviderLease {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
    }
}

fn stream_with_lease(
    mut stream: thinclaw_llm_core::StreamChunkStream,
    lease: ProviderLease,
) -> thinclaw_llm_core::StreamChunkStream {
    Box::pin(async_stream::stream! {
        use futures::StreamExt;
        let _lease = lease;
        while let Some(chunk) = stream.next().await {
            yield chunk;
        }
    })
}

fn aggregate_stream_support(supports: impl IntoIterator<Item = StreamSupport>) -> StreamSupport {
    let mut saw_simulated = false;
    for support in supports {
        match support {
            StreamSupport::Native => return StreamSupport::Native,
            StreamSupport::Simulated => saw_simulated = true,
            StreamSupport::Unsupported => {}
        }
    }
    if saw_simulated {
        StreamSupport::Simulated
    } else {
        StreamSupport::Unsupported
    }
}

fn aggregate_token_capture_support(
    supports: impl IntoIterator<Item = TokenCaptureSupport>,
) -> TokenCaptureSupport {
    let mut aggregate = TokenCaptureSupport::UNSUPPORTED;
    for support in supports {
        aggregate.exact_tokens_supported |= support.exact_tokens_supported;
        aggregate.logprobs_supported |= support.logprobs_supported;
    }
    aggregate
}

fn can_failover_stream_start(err: &LlmError) -> bool {
    is_retryable(err) || matches!(err, LlmError::StreamingUnsupported { .. })
}

/// An LLM provider that wraps multiple providers and tries each in sequence
/// on transient failures.
///
/// The first provider in the list is the primary. If it fails with a retryable
/// error, the next provider is tried, and so on. Non-retryable errors
/// (e.g. `AuthFailed`, `ContextLengthExceeded`) propagate immediately.
///
/// Providers that repeatedly fail with retryable errors are temporarily
/// placed in cooldown and skipped, reducing latency.
pub struct FailoverProvider {
    providers: Vec<Arc<dyn LlmProvider>>,
    /// Stable lease bucket ID for each provider entry. When a provider family
    /// is configured with multiple credentials, each entry receives a unique
    /// lease key so concurrent requests balance across credentials.
    lease_keys: Vec<String>,
    /// Index of the provider that last handled a request successfully.
    /// Used by `model_name()` and `cost_per_token()` so downstream cost
    /// tracking reflects the provider that actually served the request.
    last_used: AtomicUsize,
    /// Per-provider cooldown tracking (same length as `providers`).
    cooldowns: Vec<ProviderCooldown>,
    /// Reference instant for computing elapsed nanos. Shared across all
    /// cooldown timestamps so they are comparable.
    epoch: Instant,
    /// Cooldown configuration.
    cooldown_config: CooldownConfig,
    /// Concurrent-request lease tracking shared across all callers of this
    /// provider chain. This prevents one provider credential from being
    /// hammered while others sit idle.
    leases: LeaseTracker,
    /// Request-scoped provider index keyed by Tokio task ID.
    ///
    /// This allows `effective_model_name()` to report the provider that handled
    /// the *current* request, even when other concurrent requests update
    /// `last_used`.
    provider_for_task: Mutex<HashMap<tokio::task::Id, usize>>,
}

impl FailoverProvider {
    /// Create a new failover provider with default cooldown settings.
    ///
    /// Returns an error if `providers` is empty.
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Result<Self, LlmError> {
        Self::with_configs(providers, CooldownConfig::default(), LeaseConfig::default())
    }

    /// Create a new failover provider with explicit cooldown configuration.
    ///
    /// Returns an error if `providers` is empty.
    pub fn with_cooldown(
        providers: Vec<Arc<dyn LlmProvider>>,
        cooldown_config: CooldownConfig,
    ) -> Result<Self, LlmError> {
        Self::with_configs(providers, cooldown_config, LeaseConfig::default())
    }

    /// Create a new failover provider with explicit cooldown and lease
    /// configuration.
    pub fn with_configs(
        providers: Vec<Arc<dyn LlmProvider>>,
        cooldown_config: CooldownConfig,
        lease_config: LeaseConfig,
    ) -> Result<Self, LlmError> {
        let entries = providers
            .into_iter()
            .enumerate()
            .map(|(idx, provider)| ProviderLeaseEntry::new(provider, format!("provider:{idx}")))
            .collect();
        Self::with_entries(entries, cooldown_config, lease_config)
    }

    /// Create a new failover provider from explicit lease entries.
    ///
    /// Callers should use this when they want leases to operate at a finer
    /// grain than "one slot per provider", for example one slot per API key.
    pub fn with_entries(
        entries: Vec<ProviderLeaseEntry>,
        cooldown_config: CooldownConfig,
        lease_config: LeaseConfig,
    ) -> Result<Self, LlmError> {
        if entries.is_empty() {
            return Err(LlmError::RequestFailed {
                provider: "failover".to_string(),
                reason: "FailoverProvider requires at least one provider".to_string(),
            });
        }

        let mut providers = Vec::with_capacity(entries.len());
        let mut lease_keys = Vec::with_capacity(entries.len());
        for entry in entries {
            providers.push(entry.provider);
            lease_keys.push(entry.lease_key);
        }

        if providers.is_empty() {
            return Err(LlmError::RequestFailed {
                provider: "failover".to_string(),
                reason: "FailoverProvider requires at least one provider".to_string(),
            });
        }
        let provider_count = providers.len();
        let cooldowns = (0..provider_count)
            .map(|_| ProviderCooldown::new())
            .collect();
        Ok(Self {
            providers,
            lease_keys,
            last_used: AtomicUsize::new(0),
            cooldowns,
            epoch: Instant::now(),
            cooldown_config,
            leases: LeaseTracker::new(provider_count, lease_config),
            provider_for_task: Mutex::new(HashMap::new()),
        })
    }

    /// Return a process-local snapshot of the credential lease pool.
    pub fn credential_pool_health_snapshot(
        &self,
        oauth_sync_enabled: bool,
        last_sync_status: Option<String>,
    ) -> CredentialPoolHealthSnapshot {
        CredentialPoolHealthSnapshot {
            provider: self.model_name().to_string(),
            entry_count: self.providers.len(),
            max_concurrency: self.leases.config.max_concurrent,
            active_lease_count: self
                .leases
                .active
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .sum(),
            selection_strategy: self.leases.config.selection_strategy,
            oauth_sync_enabled,
            last_sync_status,
        }
    }

    /// Nanoseconds elapsed since `self.epoch`.
    ///
    /// Truncates `u128` → `u64` (wraps after ~584 years of continuous
    /// uptime). Acceptable because `epoch` is set at construction time.
    fn now_nanos(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// Current Tokio task ID if available.
    fn current_task_id() -> Option<tokio::task::Id> {
        tokio::task::try_id()
    }

    /// Bind the selected provider index to the current task.
    fn bind_provider_to_current_task(&self, provider_idx: usize) {
        let Some(task_id) = Self::current_task_id() else {
            return;
        };
        if let Ok(mut guard) = self.provider_for_task.lock() {
            guard.insert(task_id, provider_idx);
        }
    }

    /// Take and remove the provider index bound to the current task.
    fn take_bound_provider_for_current_task(&self) -> Option<usize> {
        let task_id = Self::current_task_id()?;
        self.provider_for_task
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(&task_id))
    }

    /// Try each provider in sequence until one succeeds or all fail.
    ///
    /// Providers in cooldown are skipped unless *all* providers are in
    /// cooldown, in which case the one with the oldest cooldown timestamp
    /// (most likely to have recovered) is tried.
    async fn try_providers<T, F, Fut>(&self, mut call: F) -> Result<(usize, T), LlmError>
    where
        F: FnMut(Arc<dyn LlmProvider>) -> Fut,
        Fut: Future<Output = Result<T, LlmError>>,
    {
        let now_nanos = self.now_nanos();
        let cooldown_nanos = self.cooldown_config.cooldown_duration.as_nanos() as u64;

        // Partition providers into available and cooled-down.
        let (mut available, cooled_down): (Vec<usize>, Vec<usize>) = (0..self.providers.len())
            .partition(|&i| !self.cooldowns[i].is_in_cooldown(now_nanos, cooldown_nanos));

        // Log skipped providers.
        for &i in &cooled_down {
            tracing::info!(
                provider = %self.providers[i].model_name(),
                lease_key = %self.lease_keys[i],
                "Skipping provider (in cooldown)"
            );
        }

        // Never skip ALL providers: if every provider is in cooldown, pick
        // the one with the oldest cooldown activation (most likely recovered).
        if available.is_empty() {
            let oldest = (0..self.providers.len())
                .min_by_key(|&i| {
                    self.cooldowns[i]
                        .cooldown_activated_nanos
                        .load(Ordering::Relaxed)
                })
                .ok_or_else(|| LlmError::RequestFailed {
                    provider: "failover".to_string(),
                    reason: "FailoverProvider requires at least one provider".to_string(),
                })?;
            tracing::info!(
                provider = %self.providers[oldest].model_name(),
                lease_key = %self.lease_keys[oldest],
                "All providers in cooldown, trying oldest-cooled provider"
            );
            available.push(oldest);
        }

        let mut last_error: Option<LlmError> = None;

        let ordered = self.leases.order_candidates(&available);
        let mut attempted_any = false;

        for (pos, &i) in ordered.iter().enumerate() {
            let Some(_lease) = self.leases.try_acquire(i) else {
                tracing::info!(
                    provider = %self.providers[i].model_name(),
                    lease_key = %self.lease_keys[i],
                    max_concurrent = self.leases.config.max_concurrent,
                    "Skipping provider (lease capacity reached)"
                );
                continue;
            };
            attempted_any = true;
            let provider = &self.providers[i];
            let result = call(Arc::clone(provider)).await;
            match result {
                Ok(response) => {
                    self.last_used.store(i, Ordering::Relaxed);
                    self.cooldowns[i].reset();
                    return Ok((i, response));
                }
                Err(err) => {
                    if !is_retryable(&err) {
                        return Err(err);
                    }

                    // Increment failure count; activate cooldown if threshold reached.
                    if self.cooldowns[i].record_failure(self.cooldown_config.failure_threshold) {
                        let nanos = self.now_nanos();
                        self.cooldowns[i].activate_cooldown(nanos);
                        tracing::warn!(
                            provider = %provider.model_name(),
                            lease_key = %self.lease_keys[i],
                            threshold = self.cooldown_config.failure_threshold,
                            cooldown_secs = self.cooldown_config.cooldown_duration.as_secs(),
                            "Provider entered cooldown after repeated failures"
                        );
                    }

                    // Bug 6 fix: add jitter before trying the next provider so
                    // concurrent callers don't all hammer the next provider at the
                    // exact same millisecond (thundering-herd on recovery).
                    // Jitter = 0–20% of 100ms per attempt, derived from index + pos
                    // as a cheap pseudo-random without importing the rand crate.
                    let jitter_ms = {
                        let seed = (i as u64).wrapping_mul(2654435761)
                            ^ (pos as u64).wrapping_mul(2246822519)
                            ^ self.now_nanos();
                        (seed >> 50) % 20 // 0..20 ms
                    };
                    if pos + 1 < ordered.len() {
                        let next_i = ordered[pos + 1];
                        tracing::warn!(
                            provider = %provider.model_name(),
                            lease_key = %self.lease_keys[i],
                            error = %err,
                            next_provider = %self.providers[next_i].model_name(),
                            next_lease_key = %self.lease_keys[next_i],
                            jitter_ms = jitter_ms,
                            "Provider failed with retryable error, trying next provider"
                        );
                        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
                    }
                    last_error = Some(err);
                }
            }
        }

        if !attempted_any {
            return Err(LlmError::RequestFailed {
                provider: "failover".to_string(),
                reason: format!(
                    "All providers are at lease capacity (max_concurrent={})",
                    self.leases.config.max_concurrent
                ),
            });
        }

        Err(last_error.unwrap_or_else(|| LlmError::RequestFailed {
            provider: "failover".to_string(),
            reason: "Invariant violated in FailoverProvider: providers were exhausted but no last_error was recorded (this branch should be unreachable; possible causes: no provider attempts were made or `available` was unexpectedly empty).".to_string(),
        }))
    }
}

#[async_trait]
impl LlmProvider for FailoverProvider {
    fn model_name(&self) -> &str {
        self.providers[self.last_used.load(Ordering::Relaxed)].model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.providers[self.last_used.load(Ordering::Relaxed)].cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (provider_idx, response) = self
            .try_providers(|provider| {
                let req = request.clone();
                async move { provider.complete(req).await }
            })
            .await?;
        self.bind_provider_to_current_task(provider_idx);
        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (provider_idx, response) = self
            .try_providers(|provider| {
                let req = request.clone();
                async move { provider.complete_with_tools(req).await }
            })
            .await?;
        self.bind_provider_to_current_task(provider_idx);
        Ok(response)
    }

    fn active_model_name(&self) -> String {
        self.providers[self.last_used.load(Ordering::Relaxed)].active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        for provider in &self.providers {
            provider.set_model(model)?;
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let mut all_models = Vec::new();

        for provider in &self.providers {
            match provider.list_models().await {
                Ok(models) => all_models.extend(models),
                Err(err) => {
                    tracing::warn!(
                        provider = %provider.model_name(),
                        error = %err,
                        "Failed to list models from provider, skipping"
                    );
                }
            }
        }

        all_models.sort();
        all_models.dedup();
        Ok(all_models)
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.providers[self.last_used.load(Ordering::Relaxed)]
            .model_metadata()
            .await
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.providers[self.last_used.load(Ordering::Relaxed)]
            .calculate_cost(input_tokens, output_tokens)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        // We cannot fail over mid-stream, but we can try alternate providers
        // before the stream starts.
        let now_nanos = self.now_nanos();
        let cooldown_nanos = self.cooldown_config.cooldown_duration.as_nanos() as u64;
        let (mut available, cooled_down): (Vec<usize>, Vec<usize>) = (0..self.providers.len())
            .partition(|&i| !self.cooldowns[i].is_in_cooldown(now_nanos, cooldown_nanos));
        for &idx in &cooled_down {
            tracing::info!(
                provider = %self.providers[idx].model_name(),
                lease_key = %self.lease_keys[idx],
                "Skipping streaming provider (in cooldown)"
            );
        }
        if available.is_empty()
            && let Some(oldest) = (0..self.providers.len()).min_by_key(|&i| {
                self.cooldowns[i]
                    .cooldown_activated_nanos
                    .load(Ordering::Relaxed)
            })
        {
            available.push(oldest);
        }

        let ordered = self.leases.order_candidates(&available);
        let mut attempted_any = false;
        let mut last_error: Option<LlmError> = None;
        for &idx in &ordered {
            let Some(lease) = self.leases.try_acquire(idx) else {
                tracing::info!(
                    provider = %self.providers[idx].model_name(),
                    lease_key = %self.lease_keys[idx],
                    max_concurrent = self.leases.config.max_concurrent,
                    "Skipping streaming provider (lease capacity reached)"
                );
                continue;
            };
            attempted_any = true;
            match self.providers[idx].complete_stream(request.clone()).await {
                Ok(stream) => {
                    self.last_used.store(idx, Ordering::Relaxed);
                    self.cooldowns[idx].reset();
                    self.bind_provider_to_current_task(idx);
                    return Ok(stream_with_lease(stream, lease));
                }
                Err(err) => {
                    drop(lease);
                    tracing::warn!(
                        provider = %self.providers[idx].model_name(),
                        error = %err,
                        "Streaming start failed, trying next failover provider"
                    );
                    if !can_failover_stream_start(&err) {
                        return Err(err);
                    }
                    if is_retryable(&err)
                        && self.cooldowns[idx]
                            .record_failure(self.cooldown_config.failure_threshold)
                    {
                        let nanos = self.now_nanos();
                        self.cooldowns[idx].activate_cooldown(nanos);
                    }
                    last_error = Some(err);
                }
            }
        }

        if !attempted_any {
            return Err(LlmError::RequestFailed {
                provider: "failover".to_string(),
                reason: format!(
                    "All streaming providers are at lease capacity (max_concurrent={})",
                    self.leases.config.max_concurrent
                ),
            });
        }

        Err(last_error.unwrap_or_else(|| LlmError::RequestFailed {
            provider: self.model_name().to_owned(),
            reason: "All failover providers failed to start streaming".to_string(),
        }))
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        let now_nanos = self.now_nanos();
        let cooldown_nanos = self.cooldown_config.cooldown_duration.as_nanos() as u64;
        let (mut available, cooled_down): (Vec<usize>, Vec<usize>) = (0..self.providers.len())
            .partition(|&i| !self.cooldowns[i].is_in_cooldown(now_nanos, cooldown_nanos));
        for &idx in &cooled_down {
            tracing::info!(
                provider = %self.providers[idx].model_name(),
                lease_key = %self.lease_keys[idx],
                "Skipping tool streaming provider (in cooldown)"
            );
        }
        if available.is_empty()
            && let Some(oldest) = (0..self.providers.len()).min_by_key(|&i| {
                self.cooldowns[i]
                    .cooldown_activated_nanos
                    .load(Ordering::Relaxed)
            })
        {
            available.push(oldest);
        }

        let ordered = self.leases.order_candidates(&available);
        let mut attempted_any = false;
        let mut last_error: Option<LlmError> = None;
        for &idx in &ordered {
            let Some(lease) = self.leases.try_acquire(idx) else {
                tracing::info!(
                    provider = %self.providers[idx].model_name(),
                    lease_key = %self.lease_keys[idx],
                    max_concurrent = self.leases.config.max_concurrent,
                    "Skipping tool streaming provider (lease capacity reached)"
                );
                continue;
            };
            attempted_any = true;
            match self.providers[idx]
                .complete_stream_with_tools(request.clone())
                .await
            {
                Ok(stream) => {
                    self.last_used.store(idx, Ordering::Relaxed);
                    self.cooldowns[idx].reset();
                    self.bind_provider_to_current_task(idx);
                    return Ok(stream_with_lease(stream, lease));
                }
                Err(err) => {
                    drop(lease);
                    tracing::warn!(
                        provider = %self.providers[idx].model_name(),
                        error = %err,
                        "Tool streaming start failed, trying next failover provider"
                    );
                    if !can_failover_stream_start(&err) {
                        return Err(err);
                    }
                    if is_retryable(&err)
                        && self.cooldowns[idx]
                            .record_failure(self.cooldown_config.failure_threshold)
                    {
                        let nanos = self.now_nanos();
                        self.cooldowns[idx].activate_cooldown(nanos);
                    }
                    last_error = Some(err);
                }
            }
        }

        if !attempted_any {
            return Err(LlmError::RequestFailed {
                provider: "failover".to_string(),
                reason: format!(
                    "All tool streaming providers are at lease capacity (max_concurrent={})",
                    self.leases.config.max_concurrent
                ),
            });
        }

        Err(last_error.unwrap_or_else(|| LlmError::RequestFailed {
            provider: self.model_name().to_owned(),
            reason: "All failover providers failed to start tool streaming".to_string(),
        }))
    }

    fn supports_streaming(&self) -> bool {
        self.stream_support().is_native()
    }

    fn stream_support(&self) -> StreamSupport {
        aggregate_stream_support(
            self.providers
                .iter()
                .map(|provider| provider.stream_support()),
        )
    }

    fn stream_support_for_model(&self, requested_model: Option<&str>) -> StreamSupport {
        aggregate_stream_support(
            self.providers
                .iter()
                .map(|provider| provider.stream_support_for_model(requested_model)),
        )
    }

    fn token_capture_support(&self) -> TokenCaptureSupport {
        aggregate_token_capture_support(
            self.providers
                .iter()
                .map(|provider| provider.token_capture_support()),
        )
    }

    fn token_capture_support_for_model(
        &self,
        requested_model: Option<&str>,
    ) -> TokenCaptureSupport {
        aggregate_token_capture_support(
            self.providers
                .iter()
                .map(|provider| provider.token_capture_support_for_model(requested_model)),
        )
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        if let Some(provider_idx) = self.take_bound_provider_for_current_task() {
            return self.providers[provider_idx].effective_model_name(requested_model);
        }

        self.providers[self.last_used.load(Ordering::Relaxed)].effective_model_name(requested_model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use rust_decimal_macros::dec;
    use std::sync::atomic::AtomicUsize;
    use thinclaw_llm_core::provider::{
        ChatMessage, CompletionRequest, CompletionResponse, FinishReason, StreamChunk,
        StreamChunkStream, ToolCompletionRequest, ToolCompletionResponse,
    };

    struct MockProvider {
        name: &'static str,
        complete_result: MockCompleteResult,
        stream_chunks: Vec<StreamChunk>,
        complete_calls: AtomicUsize,
    }

    #[derive(Clone)]
    enum MockCompleteResult {
        Success,
        Error,
    }

    impl MockProvider {
        fn success(name: &'static str) -> Self {
            Self {
                name,
                complete_result: MockCompleteResult::Success,
                stream_chunks: Vec::new(),
                complete_calls: AtomicUsize::new(0),
            }
        }

        fn error(name: &'static str) -> Self {
            Self {
                name,
                complete_result: MockCompleteResult::Error,
                stream_chunks: Vec::new(),
                complete_calls: AtomicUsize::new(0),
            }
        }

        fn streaming(name: &'static str, chunks: Vec<StreamChunk>) -> Self {
            Self {
                name,
                complete_result: MockCompleteResult::Success,
                stream_chunks: chunks,
                complete_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn model_name(&self) -> &str {
            self.name
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (dec!(0), dec!(0))
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            self.complete_calls.fetch_add(1, Ordering::Relaxed);
            match self.complete_result {
                MockCompleteResult::Success => Ok(CompletionResponse {
                    content: "ok".to_string(),
                    provider_model: Some(self.name.to_string()),
                    cost_usd: None,
                    thinking_content: None,
                    input_tokens: 1,
                    output_tokens: 1,
                    finish_reason: FinishReason::Stop,
                    token_capture: None,
                }),
                MockCompleteResult::Error => Err(LlmError::RequestFailed {
                    provider: self.name.to_string(),
                    reason: "mock failure".to_string(),
                }),
            }
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: self.name.to_string(),
                reason: "not used".to_string(),
            })
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamChunkStream, LlmError> {
            Ok(Box::pin(futures::stream::iter(
                self.stream_chunks.clone().into_iter().map(Ok),
            )))
        }

        fn stream_support(&self) -> StreamSupport {
            StreamSupport::Native
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequest::new(vec![ChatMessage::user("hello")])
    }

    fn failover_with_provider(provider: MockProvider, max_concurrent: usize) -> FailoverProvider {
        FailoverProvider::with_entries(
            vec![ProviderLeaseEntry::new(Arc::new(provider), "mock:1")],
            CooldownConfig::default(),
            LeaseConfig {
                max_concurrent,
                selection_strategy: LeaseSelectionStrategy::FillFirst,
            },
        )
        .unwrap()
    }

    fn done_chunk() -> StreamChunk {
        StreamChunk::Done {
            provider_model: Some("mock".to_string()),
            cost_usd: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        }
    }

    #[test]
    fn lease_config_zero_is_clamped_to_one_in_process() {
        let provider = failover_with_provider(MockProvider::success("mock"), 0);

        let snapshot = provider.credential_pool_health_snapshot(false, None);

        assert_eq!(snapshot.max_concurrency, 1);
    }

    #[test]
    fn lease_released_after_normal_completion() {
        let provider = failover_with_provider(MockProvider::success("mock"), 1);

        futures::executor::block_on(provider.complete(request())).unwrap();

        let snapshot = provider.credential_pool_health_snapshot(false, None);
        assert_eq!(snapshot.active_lease_count, 0);
    }

    #[test]
    fn lease_released_after_provider_error() {
        let provider = failover_with_provider(MockProvider::error("mock"), 1);

        let result = futures::executor::block_on(provider.complete(request()));

        assert!(result.is_err());
        let snapshot = provider.credential_pool_health_snapshot(false, None);
        assert_eq!(snapshot.active_lease_count, 0);
    }

    #[test]
    fn lease_released_after_stream_exhaustion() {
        let provider = failover_with_provider(
            MockProvider::streaming(
                "mock",
                vec![StreamChunk::Text("hello".to_string()), done_chunk()],
            ),
            1,
        );

        let mut stream = futures::executor::block_on(provider.complete_stream(request())).unwrap();
        assert_eq!(
            provider
                .credential_pool_health_snapshot(false, None)
                .active_lease_count,
            1
        );

        while futures::executor::block_on(stream.next()).is_some() {}

        let snapshot = provider.credential_pool_health_snapshot(false, None);
        assert_eq!(snapshot.active_lease_count, 0);
    }

    #[test]
    fn lease_released_when_stream_is_dropped() {
        let provider = failover_with_provider(
            MockProvider::streaming(
                "mock",
                vec![StreamChunk::Text("hello".to_string()), done_chunk()],
            ),
            1,
        );

        let stream = futures::executor::block_on(provider.complete_stream(request())).unwrap();
        assert_eq!(
            provider
                .credential_pool_health_snapshot(true, Some("synced 1 source".to_string()))
                .active_lease_count,
            1
        );

        drop(stream);

        let snapshot =
            provider.credential_pool_health_snapshot(true, Some("synced 1 source".to_string()));
        assert_eq!(snapshot.active_lease_count, 0);
        assert!(snapshot.oauth_sync_enabled);
        assert_eq!(
            snapshot.last_sync_status.as_deref(),
            Some("synced 1 source")
        );
    }
}
