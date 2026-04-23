use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use rust_decimal::prelude::ToPrimitive;

use crate::config::{Config, LlmBackend};
use crate::db::Database;
use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamChunkStream,
    ToolCompletionRequest, ToolCompletionResponse,
};
use crate::llm::provider_factory::{
    build_provider_chain, create_llm_provider, create_provider_for_catalog_entry,
};
use crate::llm::route_planner::{
    RequiredCapabilities, RoutePlanner, RoutePlannerInput, log_routing_decision,
    validate_providers_settings as validate_planner_settings,
};
use crate::llm::routing_policy::{RouteCandidate, RoutingContext, RoutingPolicy, RoutingRule};
use crate::llm::usage_tracking::{
    USAGE_TRACKING_ENDPOINT_TYPE_KEY, USAGE_TRACKING_TELEMETRY_KEY, USAGE_TRACKING_WORKLOAD_TAG_KEY,
};
use crate::llm::{
    CachedProvider, CircuitBreakerConfig, CircuitBreakerProvider, CooldownConfig, FailoverProvider,
    ResponseCacheConfig, RetryConfig, RetryProvider,
};
use crate::secrets::SecretsStore;
use crate::settings::{ProvidersSettings, RoutingMode, Settings};

#[derive(Clone)]
pub struct RuntimeStatus {
    pub revision: u64,
    pub last_error: Option<String>,
    pub primary_model: String,
    pub cheap_model: Option<String>,
    pub routing_enabled: bool,
    pub routing_mode: RoutingMode,
    pub tool_phase_synthesis_enabled: bool,
    pub tool_phase_primary_thinking_enabled: bool,
    pub primary_provider: Option<String>,
    pub fallback_chain: Vec<String>,
    /// AdvisorExecutor: max advisor calls per turn.
    pub advisor_max_calls: u32,
    /// AdvisorExecutor: automatic escalation policy.
    pub advisor_auto_escalation_mode: crate::settings::AdvisorAutoEscalationMode,
    /// AdvisorExecutor: custom advisor escalation prompt.
    pub advisor_escalation_prompt: Option<String>,
    /// Whether the advisor lane is currently usable.
    pub advisor_ready: bool,
    /// Why the advisor lane is unavailable, if known.
    pub advisor_disabled_reason: Option<String>,
    /// Resolved executor model spec selected for AdvisorExecutor diagnostics.
    pub executor_target: Option<String>,
    /// Resolved advisor model spec selected for AdvisorExecutor diagnostics.
    pub advisor_target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RouteSimulationScore {
    pub target: String,
    pub telemetry_key: Option<String>,
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
    pub policy_bias: f64,
    pub composite: f64,
}

#[derive(Debug, Clone)]
pub struct RouteSimulationResult {
    pub target: String,
    pub reason: String,
    pub fallback_chain: Vec<String>,
    pub candidate_list: Vec<String>,
    pub rejections: Vec<String>,
    pub score_breakdown: Vec<RouteSimulationScore>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone)]
struct LlmRuntimeSnapshot {
    config: Config,
    providers: ProvidersSettings,
    llm: Arc<dyn LlmProvider>,
    cheap_llm: Option<Arc<dyn LlmProvider>>,
}

type AdvisorReadyCallback = Arc<dyn Fn(bool) + Send + Sync>;

#[derive(Clone, Copy)]
enum RuntimeProviderRole {
    Primary,
    Cheap,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProviderModelRole {
    Primary,
    Cheap,
}

impl ProviderModelRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Cheap => "cheap",
        }
    }
}

struct ResolvedRoute {
    provider: Arc<dyn LlmProvider>,
    telemetry_key: String,
}

pub struct RuntimeLlmProvider {
    manager: Arc<LlmRuntimeManager>,
    role: RuntimeProviderRole,
}

impl RuntimeLlmProvider {
    fn new(manager: Arc<LlmRuntimeManager>, role: RuntimeProviderRole) -> Self {
        Self { manager, role }
    }

    fn current_provider(&self) -> Arc<dyn LlmProvider> {
        let snapshot = self.manager.snapshot();
        match self.role {
            RuntimeProviderRole::Primary => snapshot.llm,
            RuntimeProviderRole::Cheap => snapshot.cheap_llm.unwrap_or(snapshot.llm),
        }
    }

    fn routing_context(
        &self,
        messages: &[crate::llm::ChatMessage],
        has_tools: bool,
        requires_streaming: bool,
    ) -> (RoutingContext, Option<String>) {
        let estimated_input_tokens = messages
            .iter()
            .map(|m| (m.estimated_chars() / 4) as u32)
            .sum();
        let has_vision = messages.iter().any(|m| {
            m.attachments
                .iter()
                .any(|a| a.mime_type.starts_with("image/"))
        });
        let last_user_message = messages
            .iter()
            .rev()
            .find(|m| m.role == crate::llm::Role::User)
            .map(|m| m.content.trim().to_string())
            .filter(|msg| !msg.is_empty());

        (
            RoutingContext {
                estimated_input_tokens,
                has_vision,
                has_tools,
                requires_streaming,
                budget_usd: None,
            },
            last_user_message,
        )
    }

    fn provider_for_request(
        &self,
        requested_model: Option<&str>,
        routing_context: Option<&RoutingContext>,
        last_user_message: Option<String>,
    ) -> Result<ResolvedRoute, LlmError> {
        let snapshot = self.manager.snapshot();

        if let Some(model) = requested_model {
            let provider = self.manager.provider_for_model_spec(model, &snapshot)?;
            let telemetry_key = format!("unknown|unknown|{}", model.trim());
            return Ok(ResolvedRoute {
                provider,
                telemetry_key,
            });
        }

        if routing_kill_switch_enabled() {
            let (provider, target) = match self.role {
                RuntimeProviderRole::Primary => (Arc::clone(&snapshot.llm), "primary"),
                RuntimeProviderRole::Cheap => (
                    snapshot
                        .cheap_llm
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| Arc::clone(&snapshot.llm)),
                    "cheap",
                ),
            };
            let telemetry_key = self
                .manager
                .resolve_telemetry_key_for_target(target, &snapshot)
                .unwrap_or_else(|| format!("killswitch|runtime|{target}"));
            return Ok(ResolvedRoute {
                provider,
                telemetry_key,
            });
        }

        // ── RoutePlanner-driven routing (Phase 6b cutover) ──
        if snapshot.providers.smart_routing_enabled
            && let Some(ctx) = routing_context
        {
            let planner_input = RoutePlannerInput {
                required_capabilities: RequiredCapabilities::from_routing_context(ctx),
                routing_mode: snapshot.providers.routing_mode,
                routing_context: ctx.clone(),
                model_override: None,
                provider_health: self.manager.route_health_snapshot(),
                candidates: self.manager.available_route_candidates(&snapshot),
                turn_cost_usd: 0.0,
                budget_utilization: None,
                last_user_message,
                advisor_escalation_prompt: snapshot.providers.advisor_escalation_prompt.clone(),
                primary_provider_preferences: normalized_provider_pool_order(
                    &snapshot.providers,
                    ProviderModelRole::Primary,
                ),
                cheap_provider_preferences: normalized_provider_pool_order(
                    &snapshot.providers,
                    ProviderModelRole::Cheap,
                ),
            };

            if let Ok(guard) = self.manager.route_planner.read() {
                let mut decision = guard.plan(
                    &planner_input,
                    self.manager.routing_policy.read().ok().as_deref(),
                );

                // Resolve the planner target to a provider
                let primary_providers = if decision.fallbacks.is_empty() {
                    vec![decision.target.clone()]
                } else {
                    let mut targets = vec![decision.target.clone()];
                    for fb in &decision.fallbacks {
                        if !targets.contains(fb) {
                            targets.push(fb.clone());
                        }
                    }
                    targets
                };

                let provider = self
                    .manager
                    .provider_chain_for_targets(&primary_providers, &snapshot)?;
                let telemetry_key = self
                    .manager
                    .resolve_telemetry_key_for_target(&decision.target, &snapshot)
                    .or_else(|| {
                        if decision.telemetry_key.contains('|') {
                            Some(decision.telemetry_key.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| format!("unknown|unknown|{}", decision.target));
                decision.telemetry_key = telemetry_key.clone();
                log_routing_decision(&decision, snapshot.providers.routing_mode.as_str());
                return Ok(ResolvedRoute {
                    provider,
                    telemetry_key,
                });
            }
        }

        // Fallback: no routing context available or planner lock failed
        let provider = match self.role {
            RuntimeProviderRole::Primary => snapshot.llm,
            RuntimeProviderRole::Cheap => snapshot.cheap_llm.unwrap_or(snapshot.llm),
        };
        Ok(ResolvedRoute {
            provider,
            telemetry_key: "unknown|unknown|runtime_fallback".to_string(),
        })
    }

    fn resolved_completion_request(
        mut request: CompletionRequest,
        telemetry_key: &str,
        workload_tag: &str,
    ) -> CompletionRequest {
        // The runtime resolves `request.model` to a concrete provider before
        // delegating, so downstream adapters should not see a stale override.
        let endpoint_type = if request.model.is_some() {
            "model_override"
        } else {
            "runtime_routed"
        };
        request.model = None;
        request.metadata.insert(
            USAGE_TRACKING_TELEMETRY_KEY.to_string(),
            telemetry_key.to_string(),
        );
        request
            .metadata
            .entry(USAGE_TRACKING_ENDPOINT_TYPE_KEY.to_string())
            .or_insert_with(|| endpoint_type.to_string());
        request
            .metadata
            .entry(USAGE_TRACKING_WORKLOAD_TAG_KEY.to_string())
            .or_insert_with(|| workload_tag.to_string());
        request
    }

    fn resolved_tool_completion_request(
        mut request: ToolCompletionRequest,
        telemetry_key: &str,
        workload_tag: &str,
    ) -> ToolCompletionRequest {
        // The runtime resolves `request.model` to a concrete provider before
        // delegating, so downstream adapters should not see a stale override.
        let endpoint_type = if request.model.is_some() {
            "model_override"
        } else {
            "runtime_routed"
        };
        request.model = None;
        request.metadata.insert(
            USAGE_TRACKING_TELEMETRY_KEY.to_string(),
            telemetry_key.to_string(),
        );
        request
            .metadata
            .entry(USAGE_TRACKING_ENDPOINT_TYPE_KEY.to_string())
            .or_insert_with(|| endpoint_type.to_string());
        request
            .metadata
            .entry(USAGE_TRACKING_WORKLOAD_TAG_KEY.to_string())
            .or_insert_with(|| workload_tag.to_string());
        request
    }
}

#[async_trait]
impl LlmProvider for RuntimeLlmProvider {
    fn model_name(&self) -> &str {
        "runtime"
    }

    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        self.current_provider().cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (ctx, last_user_message) = self.routing_context(&request.messages, false, false);
        let route =
            self.provider_for_request(request.model.as_deref(), Some(&ctx), last_user_message)?;
        let start = Instant::now();
        let result = route
            .provider
            .complete(Self::resolved_completion_request(
                request,
                &route.telemetry_key,
                "chat_completion",
            ))
            .await;
        self.manager
            .record_route_outcome(&route.telemetry_key, start.elapsed(), result.is_ok());
        result
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (ctx, last_user_message) = self.routing_context(&request.messages, true, false);
        let route =
            self.provider_for_request(request.model.as_deref(), Some(&ctx), last_user_message)?;
        let start = Instant::now();
        let result = route
            .provider
            .complete_with_tools(Self::resolved_tool_completion_request(
                request,
                &route.telemetry_key,
                "tool_completion",
            ))
            .await;
        self.manager
            .record_route_outcome(&route.telemetry_key, start.elapsed(), result.is_ok());
        result
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let (ctx, last_user_message) = self.routing_context(&request.messages, false, true);
        let route =
            self.provider_for_request(request.model.as_deref(), Some(&ctx), last_user_message)?;
        let start = Instant::now();
        let result = route
            .provider
            .complete_stream(Self::resolved_completion_request(
                request,
                &route.telemetry_key,
                "stream_completion",
            ))
            .await;
        self.manager
            .record_route_outcome(&route.telemetry_key, start.elapsed(), result.is_ok());
        result
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let (ctx, last_user_message) = self.routing_context(&request.messages, true, true);
        let route =
            self.provider_for_request(request.model.as_deref(), Some(&ctx), last_user_message)?;
        let start = Instant::now();
        let result = route
            .provider
            .complete_stream_with_tools(Self::resolved_tool_completion_request(
                request,
                &route.telemetry_key,
                "tool_stream_completion",
            ))
            .await;
        self.manager
            .record_route_outcome(&route.telemetry_key, start.elapsed(), result.is_ok());
        result
    }

    fn supports_streaming(&self) -> bool {
        self.current_provider().supports_streaming()
    }

    fn supports_streaming_for_model(&self, requested_model: Option<&str>) -> bool {
        let snapshot = self.manager.snapshot();
        self.provider_for_request(requested_model, None, None)
            .map(|route| route.provider.supports_streaming())
            .unwrap_or_else(|_| match self.role {
                RuntimeProviderRole::Primary => snapshot.llm.supports_streaming(),
                RuntimeProviderRole::Cheap => snapshot
                    .cheap_llm
                    .unwrap_or(snapshot.llm)
                    .supports_streaming(),
            })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let snapshot = self.manager.snapshot();
        let mut models = BTreeSet::new();

        models.insert(snapshot.llm.active_model_name());
        if let Some(cheap) = snapshot.cheap_llm {
            models.insert(cheap.active_model_name());
        }

        if let Some(primary) = snapshot.providers.primary.as_deref()
            && let Some(model) = snapshot.providers.primary_model.as_deref()
        {
            models.insert(format!("{primary}/{model}"));
        }

        for entry in &snapshot.providers.fallback_chain {
            models.insert(entry.clone());
        }
        if let Some(ref cheap) = snapshot.providers.cheap_model {
            models.insert(cheap.clone());
        }
        for (slug, slots) in &snapshot.providers.provider_models {
            if let Some(model) = slots.primary.as_deref() {
                models.insert(format!("{slug}/{model}"));
            }
            if let Some(model) = slots.cheap.as_deref() {
                models.insert(format!("{slug}/{model}"));
            }
        }
        for (slug, allowed) in &snapshot.providers.allowed_models {
            for model in allowed {
                models.insert(format!("{slug}/{model}"));
            }
        }

        Ok(models.into_iter().collect())
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.current_provider().model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        if let Some(model) = requested_model {
            return model.to_string();
        }
        self.active_model_name()
    }

    fn active_model_name(&self) -> String {
        let snapshot = self.manager.snapshot();
        match self.role {
            RuntimeProviderRole::Primary => snapshot
                .providers
                .primary
                .as_ref()
                .zip(snapshot.providers.primary_model.as_ref())
                .map(|(provider, model)| format!("{provider}/{model}"))
                .unwrap_or_else(|| snapshot.llm.active_model_name()),
            RuntimeProviderRole::Cheap => snapshot
                .providers
                .cheap_model
                .clone()
                .or_else(|| snapshot.cheap_llm.map(|llm| llm.active_model_name()))
                .unwrap_or_else(|| snapshot.llm.active_model_name()),
        }
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        Err(LlmError::RequestFailed {
            provider: "runtime".to_string(),
            reason: format!(
                "Runtime model switching is conversation-scoped in ThinClaw. Use llm_select or a per-request model override instead of set_model('{}').",
                model
            ),
        })
    }
}

pub struct LlmRuntimeManager {
    user_id: String,
    db: Option<Arc<dyn Database>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    toml_path: Option<PathBuf>,
    snapshot: RwLock<LlmRuntimeSnapshot>,
    pub routing_policy: Arc<RwLock<RoutingPolicy>>,
    pub route_planner: Arc<RwLock<RoutePlanner>>,
    /// Prebuilt per-target providers (reused across requests).
    target_provider_cache: RwLock<HashMap<String, Arc<dyn LlmProvider>>>,
    /// Prebuilt failover/retry chains keyed by ordered target sequence.
    chain_provider_cache: RwLock<HashMap<String, Arc<dyn LlmProvider>>>,
    /// Static candidate metadata + costs, rebuilt on startup/reload.
    route_candidate_cache: RwLock<Vec<RouteCandidate>>,
    /// Live route-health EMA keyed by canonical telemetry key.
    route_health: RwLock<HashMap<String, f64>>,
    /// Last observed dynamic pricing revision used to hydrate candidate costs.
    dynamic_pricing_revision_seen: AtomicU64,
    revision: AtomicU64,
    last_error: RwLock<Option<String>>,
    advisor_ready_callback: RwLock<Option<AdvisorReadyCallback>>,
}

impl LlmRuntimeManager {
    pub fn new(
        config: Config,
        providers: ProvidersSettings,
        db: Option<Arc<dyn Database>>,
        secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
        user_id: impl Into<String>,
        toml_path: Option<PathBuf>,
    ) -> Result<Arc<Self>, LlmError> {
        let providers = derive_runtime_defaults_from_parts(
            providers,
            legacy_primary_slug_from_config(&config),
            Some(config.llm.primary_model().to_string()),
        );
        let (llm, cheap_llm) = build_provider_chain(&config.llm, Some(&providers))?;
        let routing_policy = Arc::new(RwLock::new(build_routing_policy(&providers)));
        let route_planner = Arc::new(RwLock::new(RoutePlanner::new(
            providers.smart_routing_cascade,
            providers.tool_phase_synthesis_enabled,
            providers.advisor_max_calls,
        )));

        // Emit planner config validation warnings
        for warning in validate_planner_settings(&providers) {
            tracing::warn!("[route_planner] Config: {}", warning);
        }

        let manager = Arc::new(Self {
            user_id: user_id.into(),
            db,
            secrets_store,
            toml_path,
            snapshot: RwLock::new(LlmRuntimeSnapshot {
                config,
                providers: providers.clone(),
                llm,
                cheap_llm,
            }),
            routing_policy,
            route_planner,
            target_provider_cache: RwLock::new(HashMap::new()),
            chain_provider_cache: RwLock::new(HashMap::new()),
            route_candidate_cache: RwLock::new(Vec::new()),
            route_health: RwLock::new(HashMap::new()),
            dynamic_pricing_revision_seen: AtomicU64::new(
                crate::llm::costs::dynamic_pricing_revision(),
            ),
            revision: AtomicU64::new(1),
            last_error: RwLock::new(None),
            advisor_ready_callback: RwLock::new(None),
        });
        manager.refresh_route_caches()?;
        Ok(manager)
    }

    pub fn set_advisor_ready_callback<F>(&self, callback: F)
    where
        F: Fn(bool) + Send + Sync + 'static,
    {
        let callback: AdvisorReadyCallback = Arc::new(callback);
        if let Ok(mut slot) = self.advisor_ready_callback.write() {
            *slot = Some(Arc::clone(&callback));
        }
        callback(self.status().advisor_ready);
    }

    fn notify_advisor_ready_callback(&self) {
        let callback = self
            .advisor_ready_callback
            .read()
            .ok()
            .and_then(|slot| slot.clone());
        if let Some(callback) = callback {
            callback(self.status().advisor_ready);
        }
    }

    pub fn primary_handle(self: &Arc<Self>) -> Arc<dyn LlmProvider> {
        Arc::new(RuntimeLlmProvider::new(
            Arc::clone(self),
            RuntimeProviderRole::Primary,
        ))
    }

    pub fn cheap_handle(self: &Arc<Self>) -> Arc<dyn LlmProvider> {
        Arc::new(RuntimeLlmProvider::new(
            Arc::clone(self),
            RuntimeProviderRole::Cheap,
        ))
    }

    fn snapshot(&self) -> LlmRuntimeSnapshot {
        self.snapshot
            .read()
            .expect("runtime snapshot lock poisoned")
            .clone()
    }

    pub fn status(&self) -> RuntimeStatus {
        let snapshot = self.snapshot();
        let advisor_status = self.advisor_status_decision(&snapshot);
        let executor_target = advisor_status
            .as_ref()
            .and_then(|decision| decision.executor_target.as_deref())
            .and_then(|target| {
                self.resolve_route_model_spec(target, &snapshot)
                    .or_else(|| Some(target.to_string()))
            });
        let advisor_target = advisor_status
            .as_ref()
            .and_then(|decision| decision.advisor_target.as_deref())
            .and_then(|target| {
                self.resolve_route_model_spec(target, &snapshot)
                    .or_else(|| Some(target.to_string()))
            });
        RuntimeStatus {
            revision: self.revision.load(Ordering::Relaxed),
            last_error: self.last_error.read().ok().and_then(|guard| guard.clone()),
            primary_model: snapshot
                .providers
                .primary
                .as_ref()
                .zip(snapshot.providers.primary_model.as_ref())
                .map(|(provider, model)| format!("{provider}/{model}"))
                .unwrap_or_else(|| snapshot.llm.active_model_name()),
            cheap_model: snapshot.providers.cheap_model.clone(),
            routing_enabled: snapshot.providers.smart_routing_enabled,
            routing_mode: snapshot.providers.routing_mode,
            tool_phase_synthesis_enabled: snapshot.providers.tool_phase_synthesis_enabled,
            tool_phase_primary_thinking_enabled: snapshot
                .providers
                .tool_phase_primary_thinking_enabled,
            primary_provider: snapshot.providers.primary.clone(),
            fallback_chain: snapshot.providers.fallback_chain.clone(),
            advisor_max_calls: snapshot.providers.advisor_max_calls,
            advisor_auto_escalation_mode: snapshot.providers.advisor_auto_escalation_mode,
            advisor_escalation_prompt: snapshot.providers.advisor_escalation_prompt.clone(),
            advisor_ready: advisor_status
                .as_ref()
                .map(|decision| decision.advisor_ready)
                .unwrap_or(false),
            advisor_disabled_reason: advisor_status
                .as_ref()
                .and_then(|decision| decision.advisor_disabled_reason.clone())
                .or_else(|| {
                    (snapshot.providers.routing_mode == RoutingMode::AdvisorExecutor
                        && !snapshot.providers.smart_routing_enabled)
                        .then_some(
                            "advisor routing is disabled because smart routing is turned off"
                                .to_string(),
                        )
                }),
            executor_target,
            advisor_target,
        }
    }

    pub async fn reload(&self) -> Result<(), LlmError> {
        if let Some(ref secrets) = self.secrets_store {
            let _ = crate::config::refresh_secrets(secrets.as_ref(), &self.user_id).await;
        }

        let reload_result = async {
            let (config, providers) =
                self.load_runtime_inputs()
                    .await
                    .map_err(|reason| LlmError::RequestFailed {
                        provider: "runtime".to_string(),
                        reason,
                    })?;
            let (llm, cheap_llm) = build_provider_chain(&config.llm, Some(&providers))?;
            let policy = build_routing_policy(&providers);
            Ok::<_, LlmError>((config, providers, llm, cheap_llm, policy))
        }
        .await;

        let (config, providers, llm, cheap_llm, policy) = match reload_result {
            Ok(loaded) => loaded,
            Err(err) => {
                if let Ok(mut last_error) = self.last_error.write() {
                    *last_error = Some(err.to_string());
                }
                return Err(err);
            }
        };

        {
            let mut snapshot = self
                .snapshot
                .write()
                .expect("runtime snapshot lock poisoned");
            *snapshot = LlmRuntimeSnapshot {
                config,
                providers: providers.clone(),
                llm,
                cheap_llm,
            };
        }
        if let Ok(mut routing_policy) = self.routing_policy.write() {
            *routing_policy = policy;
        }
        if let Ok(mut planner) = self.route_planner.write() {
            planner.update_config(
                providers.smart_routing_cascade,
                providers.tool_phase_synthesis_enabled,
                providers.advisor_max_calls,
            );
        }
        self.refresh_route_caches()?;
        // Emit planner config validation warnings on reload
        for warning in validate_planner_settings(&providers) {
            tracing::warn!("[route_planner] Config: {}", warning);
        }
        if let Ok(mut last_error) = self.last_error.write() {
            *last_error = None;
        }
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.notify_advisor_ready_callback();
        Ok(())
    }

    async fn load_runtime_inputs(&self) -> Result<(Config, ProvidersSettings), String> {
        if let Some(ref db) = self.db {
            let map = db
                .get_all_settings(&self.user_id)
                .await
                .map_err(|e| format!("Failed to load settings from DB: {e}"))?;
            let mut settings = Settings::from_db_map(&map);
            if let Some(ref toml_path) = self.toml_path
                && let Ok(Some(toml_settings)) = Settings::load_toml(toml_path)
            {
                settings.merge_from(&toml_settings);
            }
            let config =
                Config::from_db_with_toml(db.as_ref(), &self.user_id, self.toml_path.as_deref())
                    .await
                    .map_err(|e| format!("Failed to resolve config from DB: {e}"))?;
            let providers = normalize_providers_settings(&settings);
            return Ok((config, providers));
        }

        let mut settings = Settings::load();
        if let Some(ref toml_path) = self.toml_path
            && let Ok(Some(toml_settings)) = Settings::load_toml(toml_path)
        {
            settings.merge_from(&toml_settings);
        }
        let config = Config::from_env_with_toml(self.toml_path.as_deref())
            .await
            .map_err(|e| format!("Failed to resolve config from env: {e}"))?;
        let providers = normalize_providers_settings(&settings);
        Ok((config, providers))
    }

    fn route_health_snapshot(&self) -> HashMap<String, f64> {
        self.route_health
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn record_route_outcome(
        &self,
        telemetry_key: &str,
        latency: std::time::Duration,
        success: bool,
    ) {
        if let Ok(mut health) = self.route_health.try_write() {
            let current = health.get(telemetry_key).copied().unwrap_or(0.8);
            let target = if success { 1.0 } else { 0.0 };
            let alpha = if success { 0.12 } else { 0.35 };
            let next = (1.0 - alpha) * current + alpha * target;
            health.insert(telemetry_key.to_string(), next.clamp(0.05, 1.0));
        }

        if let Ok(mut policy) = self.routing_policy.try_write() {
            policy.record_latency(telemetry_key, latency.as_millis() as f64);
        }
    }

    fn refresh_route_caches(&self) -> Result<(), LlmError> {
        let snapshot = self.snapshot();
        let targets = self.gather_route_targets(&snapshot);

        let mut target_cache = HashMap::new();
        for target in &targets {
            match self.direct_provider_for_route_target_uncached(target, &snapshot) {
                Ok(provider) => {
                    target_cache.insert(target.clone(), provider);
                }
                Err(err) => {
                    tracing::warn!(
                        target = %target,
                        error = %err,
                        "Skipping uncached route target during cache rebuild"
                    );
                }
            }
        }

        let mut candidates = Vec::new();
        for target in &targets {
            if target_cache.contains_key(target) {
                candidates.push(self.build_route_candidate(target, &snapshot));
            } else {
                tracing::warn!(
                    target = %target,
                    "Skipping route candidate for unresolved target"
                );
            }
        }

        if let Ok(mut guard) = self.target_provider_cache.write() {
            *guard = target_cache;
        }
        if let Ok(mut guard) = self.chain_provider_cache.write() {
            guard.clear();
        }
        if let Ok(mut guard) = self.route_candidate_cache.write() {
            *guard = candidates;
        }
        self.dynamic_pricing_revision_seen.store(
            crate::llm::costs::dynamic_pricing_revision(),
            Ordering::Relaxed,
        );

        Ok(())
    }

    fn gather_route_targets(&self, snapshot: &LlmRuntimeSnapshot) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut targets = Vec::new();
        let mut push = |target: String| {
            if seen.insert(target.clone()) {
                targets.push(target);
            }
        };

        push("primary".to_string());

        if !provider_slot_selectors(&snapshot.providers, ProviderModelRole::Cheap).is_empty()
            || snapshot.providers.cheap_model.is_some()
            || snapshot.cheap_llm.is_some()
        {
            push("cheap".to_string());
        }

        for target in &snapshot.providers.fallback_chain {
            push(target.clone());
        }

        for target in policy_rule_targets(&snapshot.providers.policy_rules) {
            push(target);
        }

        for slug in &snapshot.providers.enabled {
            let primary_selector = provider_slot_selector(slug, ProviderModelRole::Primary);
            if provider_role_target(&snapshot.providers, slug, ProviderModelRole::Primary).is_some()
            {
                push(primary_selector);
            }
            let cheap_selector = provider_slot_selector(slug, ProviderModelRole::Cheap);
            if provider_role_target(&snapshot.providers, slug, ProviderModelRole::Cheap).is_some() {
                push(cheap_selector);
            }
        }

        targets
    }

    fn build_route_candidate(&self, target: &str, snapshot: &LlmRuntimeSnapshot) -> RouteCandidate {
        let logical_role = logical_role_for_target(target);
        let (provider_slug, model_id) = self.resolve_route_identity(target, snapshot);
        let telemetry_key = Some(format!(
            "{}|{}|{}",
            logical_role,
            provider_slug.as_deref().unwrap_or("unknown"),
            model_id.as_deref().unwrap_or(target)
        ));

        let (cost_per_m_usd, cost_stale) = self.route_target_cost_per_m_usd(target, snapshot);
        let capabilities =
            capability_metadata_for_route(provider_slug.as_deref(), model_id.as_deref());

        RouteCandidate::new(target.to_string(), cost_per_m_usd)
            .with_identity(provider_slug, model_id)
            .with_telemetry_key(telemetry_key)
            .with_capabilities(capabilities)
            .with_cost_stale(cost_stale)
    }

    fn resolve_route_identity(
        &self,
        target: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> (Option<String>, Option<String>) {
        let spec = self.resolve_route_model_spec(target, snapshot);
        match spec {
            Some(spec) => {
                if let Some((provider, model)) = spec.split_once('/') {
                    (Some(provider.to_string()), Some(model.to_string()))
                } else {
                    (None, Some(spec))
                }
            }
            None => (None, None),
        }
    }

    fn resolve_route_model_spec(
        &self,
        target: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Option<String> {
        match target {
            "primary" => provider_slot_selectors(&snapshot.providers, ProviderModelRole::Primary)
                .into_iter()
                .next()
                .and_then(|selector| {
                    parse_provider_slot_selector(&selector).and_then(|(slug, role)| {
                        provider_role_target(&snapshot.providers, slug, role)
                    })
                })
                .or_else(|| {
                    snapshot
                        .providers
                        .primary
                        .as_ref()
                        .zip(snapshot.providers.primary_model.as_ref())
                        .map(|(provider, model)| format!("{provider}/{model}"))
                })
                .or_else(|| Some(snapshot.llm.active_model_name())),
            "cheap" => provider_slot_selectors(&snapshot.providers, ProviderModelRole::Cheap)
                .into_iter()
                .next()
                .and_then(|selector| {
                    parse_provider_slot_selector(&selector).and_then(|(slug, role)| {
                        provider_role_target(&snapshot.providers, slug, role)
                    })
                })
                .or_else(|| snapshot.providers.cheap_model.clone())
                .or_else(|| {
                    snapshot
                        .cheap_llm
                        .as_ref()
                        .map(|llm| llm.active_model_name())
                })
                .or_else(|| Some(snapshot.llm.active_model_name())),
            other if parse_provider_slot_selector(other).is_some() => {
                let (provider, role) = parse_provider_slot_selector(other)?;
                provider_role_target(&snapshot.providers, provider, role)
            }
            other => Some(other.to_string()),
        }
    }

    fn resolve_telemetry_key_for_target(
        &self,
        target: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Option<String> {
        let (provider_slug, model_id) = self.resolve_route_identity(target, snapshot);
        let model_id = model_id?;
        Some(format!(
            "{}|{}|{}",
            logical_role_for_target(target),
            provider_slug.as_deref().unwrap_or("unknown"),
            model_id
        ))
    }

    fn provider_for_route_target(
        &self,
        target: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        match target {
            "primary" => {
                let targets =
                    provider_slot_selectors(&snapshot.providers, ProviderModelRole::Primary);
                if targets.is_empty() {
                    Ok(snapshot.llm.clone())
                } else {
                    self.provider_chain_for_targets(&targets, snapshot)
                }
            }
            "cheap" => {
                let targets =
                    provider_slot_selectors(&snapshot.providers, ProviderModelRole::Cheap);
                if targets.is_empty() {
                    Ok(snapshot
                        .cheap_llm
                        .clone()
                        .unwrap_or_else(|| snapshot.llm.clone()))
                } else {
                    self.provider_chain_for_targets(&targets, snapshot)
                }
            }
            other if parse_provider_slot_selector(other).is_some() => {
                let (provider, role) =
                    parse_provider_slot_selector(other).expect("slot selector checked above");
                self.provider_for_provider_slot(provider, role, snapshot)
            }
            other => self.provider_for_model_spec(other, snapshot),
        }
    }

    fn direct_provider_for_route_target(
        &self,
        target: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if let Ok(cache) = self.target_provider_cache.read()
            && let Some(provider) = cache.get(target)
        {
            return Ok(provider.clone());
        }
        self.direct_provider_for_route_target_uncached(target, snapshot)
    }

    fn direct_provider_for_route_target_uncached(
        &self,
        target: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        match target {
            "primary" => {
                if let Some(target) =
                    provider_slot_selectors(&snapshot.providers, ProviderModelRole::Primary)
                        .into_iter()
                        .next()
                {
                    return self.direct_provider_for_route_target(&target, snapshot);
                }
                let provider = create_llm_provider(&snapshot.config.llm)?;
                Ok(Self::wrap_runtime_provider_with_retry(provider, snapshot))
            }
            "cheap" => {
                if let Some(target) =
                    provider_slot_selectors(&snapshot.providers, ProviderModelRole::Cheap)
                        .into_iter()
                        .next()
                {
                    self.direct_provider_for_route_target(&target, snapshot)
                } else {
                    self.direct_provider_for_route_target("primary", snapshot)
                }
            }
            other if parse_provider_slot_selector(other).is_some() => {
                let (provider, role) =
                    parse_provider_slot_selector(other).expect("slot selector checked above");
                self.provider_for_provider_slot(provider, role, snapshot)
            }
            other => self.provider_for_model_spec(other, snapshot),
        }
    }

    fn provider_chain_for_targets(
        &self,
        targets: &[String],
        snapshot: &LlmRuntimeSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if targets.len() == 1
            && let Ok(cache) = self.target_provider_cache.read()
            && let Some(provider) = cache.get(&targets[0])
        {
            return Ok(provider.clone());
        }

        let chain_key = targets.join("->");
        if let Ok(cache) = self.chain_provider_cache.read()
            && let Some(provider) = cache.get(&chain_key)
        {
            return Ok(provider.clone());
        }

        let mut providers = Vec::new();

        for target in targets {
            match self.direct_provider_for_route_target(target, snapshot) {
                Ok(provider) => providers.push(provider),
                Err(err) => {
                    tracing::warn!(
                        target = %target,
                        error = %err,
                        "Skipping unusable routing target"
                    );
                }
            }
        }

        if providers.is_empty() {
            return Err(LlmError::RequestFailed {
                provider: "runtime".to_string(),
                reason: "No usable providers were available for the requested route".to_string(),
            });
        }

        if providers.len() == 1 {
            let provider =
                Self::wrap_runtime_provider_with_reliability(providers.remove(0), snapshot);
            if let Ok(mut cache) = self.chain_provider_cache.write() {
                cache.insert(chain_key, provider.clone());
            }
            return Ok(provider);
        }

        let rel = &snapshot.config.llm.reliability;
        let cooldown = CooldownConfig {
            failure_threshold: rel.failover_cooldown_threshold,
            cooldown_duration: std::time::Duration::from_secs(rel.failover_cooldown_secs),
        };
        let provider: Arc<dyn LlmProvider> = Arc::new(
            FailoverProvider::with_cooldown(providers, cooldown).map_err(|reason| {
                LlmError::RequestFailed {
                    provider: "runtime".to_string(),
                    reason: format!("Failed to build routing chain: {}", reason),
                }
            })?,
        );

        let provider = Self::wrap_runtime_provider_with_reliability(provider, snapshot);

        if let Ok(mut cache) = self.chain_provider_cache.write() {
            cache.insert(chain_key, provider.clone());
        }

        Ok(provider)
    }

    fn wrap_runtime_provider_with_retry(
        provider: Arc<dyn LlmProvider>,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Arc<dyn LlmProvider> {
        let rel = &snapshot.config.llm.reliability;
        if rel.max_retries > 0 {
            Arc::new(RetryProvider::new(
                provider,
                RetryConfig {
                    max_retries: rel.max_retries,
                },
            )) as Arc<dyn LlmProvider>
        } else {
            provider
        }
    }

    fn wrap_runtime_provider_with_reliability(
        provider: Arc<dyn LlmProvider>,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Arc<dyn LlmProvider> {
        let rel = &snapshot.config.llm.reliability;
        let mut wrapped = Self::wrap_runtime_provider_with_retry(provider, snapshot);

        if let Some(threshold) = rel.circuit_breaker_threshold {
            let cb_config = CircuitBreakerConfig {
                failure_threshold: threshold,
                recovery_timeout: std::time::Duration::from_secs(rel.circuit_breaker_recovery_secs),
                ..CircuitBreakerConfig::default()
            };
            wrapped = Arc::new(CircuitBreakerProvider::new(wrapped, cb_config));
        }

        if rel.response_cache_enabled {
            let rc_config = ResponseCacheConfig {
                ttl: std::time::Duration::from_secs(rel.response_cache_ttl_secs),
                max_entries: rel.response_cache_max_entries,
            };
            wrapped = Arc::new(CachedProvider::new(wrapped, rc_config));
        }

        wrapped
    }

    fn provider_for_provider_slot(
        &self,
        provider: &str,
        role: ProviderModelRole,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        let target =
            provider_role_target(&snapshot.providers, provider, role).ok_or_else(|| {
                LlmError::RequestFailed {
                    provider: "runtime".to_string(),
                    reason: format!(
                        "No {} model is configured for provider '{}'",
                        role.as_str(),
                        provider
                    ),
                }
            })?;
        self.provider_for_model_spec(&target, snapshot)
    }

    fn provider_for_model_spec(
        &self,
        spec: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Ok(snapshot.llm.clone());
        }

        if matches!(spec, "primary" | "cheap") {
            return self.provider_for_route_target(spec, snapshot);
        }

        if let Some((provider, role)) = parse_provider_slot_selector(spec) {
            return self.provider_for_provider_slot(provider, role, snapshot);
        }

        if let Some((provider, model)) = spec.split_once('/') {
            let provider = create_provider_for_runtime_slug(provider, model, &snapshot.config.llm)?;
            return Ok(Self::wrap_runtime_provider_with_retry(provider, snapshot));
        }

        if let Some(primary_provider) = snapshot.providers.primary.as_deref() {
            let provider =
                create_provider_for_runtime_slug(primary_provider, spec, &snapshot.config.llm)?;
            return Ok(Self::wrap_runtime_provider_with_retry(provider, snapshot));
        }

        let mut llm_config = snapshot.config.llm.clone();
        apply_model_override(&mut llm_config, spec);
        let provider = create_llm_provider(&llm_config)?;
        Ok(Self::wrap_runtime_provider_with_retry(provider, snapshot))
    }

    pub fn advisor_config_for_messages(
        &self,
        messages: &[crate::llm::ChatMessage],
    ) -> Option<crate::llm::route_planner::AdvisorConfig> {
        let snapshot = self.snapshot();
        let estimated_input_tokens = messages
            .iter()
            .map(|m| (m.estimated_chars() / 4) as u32)
            .sum();
        let has_vision = messages.iter().any(|m| {
            m.attachments
                .iter()
                .any(|a| a.mime_type.starts_with("image/"))
        });
        let last_user_message = messages
            .iter()
            .rev()
            .find(|m| m.role == crate::llm::Role::User)
            .map(|m| m.content.trim().to_string())
            .filter(|msg| !msg.is_empty());

        let ctx = RoutingContext {
            estimated_input_tokens,
            has_vision,
            has_tools: true,
            requires_streaming: false,
            budget_usd: None,
        };

        self.route_decision_for_context(&snapshot, ctx, last_user_message)
            .and_then(|decision| decision.advisor)
    }

    pub fn provider_handle_for_target(
        &self,
        target: &str,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        let snapshot = self.snapshot();
        self.provider_for_route_target(target, &snapshot)
    }

    pub fn simulate_route(&self, ctx: RoutingContext) -> (String, String) {
        self.simulate_route_for_prompt(ctx, None)
    }

    pub fn simulate_route_for_prompt(
        &self,
        ctx: RoutingContext,
        prompt: Option<&str>,
    ) -> (String, String) {
        let simulation = self.simulate_route_details(ctx, prompt);
        (simulation.target, simulation.reason)
    }

    pub fn simulate_route_details(
        &self,
        ctx: RoutingContext,
        prompt: Option<&str>,
    ) -> RouteSimulationResult {
        let snapshot = self.snapshot();
        if !snapshot.providers.smart_routing_enabled {
            return RouteSimulationResult {
                target: "primary".to_string(),
                reason: "Routing disabled".to_string(),
                fallback_chain: Vec::new(),
                candidate_list: Vec::new(),
                rejections: Vec::new(),
                score_breakdown: Vec::new(),
                diagnostics: Vec::new(),
            };
        }

        if let Some(decision) =
            self.route_decision_for_context(&snapshot, ctx.clone(), prompt.map(ToOwned::to_owned))
        {
            let reason = if decision.fallbacks.is_empty() {
                decision.reason
            } else {
                format!(
                    "{}. Fallbacks: {}",
                    decision.reason,
                    decision.fallbacks.join(" -> ")
                )
            };
            RouteSimulationResult {
                target: decision.target,
                reason,
                fallback_chain: decision.fallbacks,
                candidate_list: decision.candidate_list,
                rejections: decision
                    .rejections
                    .into_iter()
                    .map(|rejection| format!("{}: {}", rejection.target, rejection.reason))
                    .collect(),
                score_breakdown: decision
                    .score_breakdown
                    .into_iter()
                    .map(|score| RouteSimulationScore {
                        target: score.target,
                        telemetry_key: score.telemetry_key,
                        quality: score.breakdown.quality,
                        cost: score.breakdown.cost,
                        latency: score.breakdown.latency,
                        health: score.breakdown.health,
                        policy_bias: score.breakdown.policy_bias,
                        composite: score.breakdown.composite,
                    })
                    .collect(),
                diagnostics: decision.diagnostics,
            }
        } else {
            RouteSimulationResult {
                target: "primary".to_string(),
                reason: "Planner lock unavailable".to_string(),
                fallback_chain: Vec::new(),
                candidate_list: Vec::new(),
                rejections: Vec::new(),
                score_breakdown: Vec::new(),
                diagnostics: Vec::new(),
            }
        }
    }

    fn advisor_status_decision(
        &self,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Option<crate::llm::route_planner::RouteDecision> {
        if snapshot.providers.routing_mode != RoutingMode::AdvisorExecutor
            || !snapshot.providers.smart_routing_enabled
        {
            return None;
        }

        self.route_decision_for_context(
            snapshot,
            RoutingContext {
                estimated_input_tokens: 1024,
                has_vision: false,
                has_tools: true,
                requires_streaming: false,
                budget_usd: None,
            },
            Some("advisor readiness probe".to_string()),
        )
    }

    fn route_decision_for_context(
        &self,
        snapshot: &LlmRuntimeSnapshot,
        ctx: RoutingContext,
        last_user_message: Option<String>,
    ) -> Option<crate::llm::route_planner::RouteDecision> {
        let planner_input = RoutePlannerInput {
            required_capabilities: RequiredCapabilities::from_routing_context(&ctx),
            routing_mode: snapshot.providers.routing_mode,
            routing_context: ctx,
            model_override: None,
            provider_health: self.route_health_snapshot(),
            candidates: self.available_route_candidates(snapshot),
            turn_cost_usd: 0.0,
            budget_utilization: None,
            last_user_message,
            advisor_escalation_prompt: snapshot.providers.advisor_escalation_prompt.clone(),
            primary_provider_preferences: normalized_provider_pool_order(
                &snapshot.providers,
                ProviderModelRole::Primary,
            ),
            cheap_provider_preferences: normalized_provider_pool_order(
                &snapshot.providers,
                ProviderModelRole::Cheap,
            ),
        };

        self.route_planner
            .read()
            .ok()
            .map(|planner| planner.plan(&planner_input, self.routing_policy.read().ok().as_deref()))
    }

    fn available_route_candidates(&self, snapshot: &LlmRuntimeSnapshot) -> Vec<RouteCandidate> {
        self.refresh_cached_candidate_costs_if_needed(snapshot);

        let mut candidates = self
            .route_candidate_cache
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default();

        let health = self.route_health.read().ok();
        let routing_policy = self.routing_policy.read().ok();
        for candidate in &mut candidates {
            if let Some(ref key) = candidate.telemetry_key {
                candidate.health = health.as_ref().and_then(|map| map.get(key).copied());
                candidate.latency_p50_ms = routing_policy
                    .as_ref()
                    .and_then(|policy| policy.latency_tracker().get_latency(key));
            } else {
                candidate.health = health
                    .as_ref()
                    .and_then(|map| map.get(&candidate.target).copied());
                candidate.latency_p50_ms = routing_policy
                    .as_ref()
                    .and_then(|policy| policy.latency_tracker().get_latency(&candidate.target));
            }
        }
        if candidates.is_empty() {
            self.gather_route_targets(snapshot)
                .into_iter()
                .map(|target| self.build_route_candidate(&target, snapshot))
                .collect()
        } else {
            candidates
        }
    }

    fn refresh_cached_candidate_costs_if_needed(&self, snapshot: &LlmRuntimeSnapshot) {
        let latest_revision = crate::llm::costs::dynamic_pricing_revision();
        let seen_revision = self.dynamic_pricing_revision_seen.load(Ordering::Relaxed);
        if latest_revision == seen_revision {
            return;
        }

        if let Ok(mut cache) = self.route_candidate_cache.write() {
            for candidate in cache.iter_mut() {
                let (cost_per_m_usd, cost_stale) =
                    self.route_target_cost_per_m_usd(&candidate.target, snapshot);
                candidate.cost_per_m_usd = cost_per_m_usd;
                candidate.cost_stale = cost_stale;
            }
            self.dynamic_pricing_revision_seen
                .store(latest_revision, Ordering::Relaxed);
        }
    }

    fn route_target_cost_per_m_usd(
        &self,
        target: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> (Option<f64>, bool) {
        let Some(spec) = self.resolve_route_model_spec(target, snapshot) else {
            return (None, false);
        };
        let Some((input_cost, output_cost, source)) =
            crate::llm::costs::model_cost_with_source(&spec)
        else {
            return (None, false);
        };
        let cost_stale = matches!(source, crate::llm::costs::CostSource::Dynamic)
            && crate::llm::costs::dynamic_pricing_is_stale(std::time::Duration::from_secs(
                48 * 3600,
            ));
        (
            ((input_cost + output_cost) * rust_decimal::Decimal::from(1_000_000u64)).to_f64(),
            cost_stale,
        )
    }
}

fn logical_role_for_target(target: &str) -> &'static str {
    if target == "cheap" || target.ends_with("@cheap") {
        "cheap"
    } else if target == "primary" || target.ends_with("@primary") {
        "primary"
    } else {
        "direct"
    }
}

fn capability_metadata_for_route(
    provider_slug: Option<&str>,
    model_id: Option<&str>,
) -> crate::llm::routing_policy::ProviderCapabilitiesMetadata {
    let mut metadata = crate::llm::routing_policy::ProviderCapabilitiesMetadata::default();
    if let Some(model_id) = model_id {
        let compat = crate::config::model_compat::find_model(model_id).or_else(|| {
            model_id
                .split_once('/')
                .and_then(|(_, model)| crate::config::model_compat::find_model(model))
        });
        if let Some(compat) = compat {
            metadata.supports_streaming = Some(compat.supports_streaming);
            metadata.supports_tools = Some(compat.supports_tools);
            metadata.supports_vision = Some(compat.supports_vision);
            metadata.supports_thinking = Some(compat.supports_thinking);
            metadata.max_context_tokens = Some(compat.context_window);
        }
    }

    if let Some(slug) = provider_slug
        && let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug)
    {
        if metadata.supports_streaming.is_none() {
            metadata.supports_streaming = Some(endpoint.supports_streaming);
        }
        if metadata.max_context_tokens.is_none() {
            metadata.max_context_tokens = Some(endpoint.default_context_size);
        }
    }

    // Explicitly-known limitation: Perplexity models currently do not expose
    // function/tool calling in ThinClaw's routing layer.
    if provider_slug == Some("perplexity") {
        metadata.supports_tools = Some(false);
    }

    metadata
}

fn routing_kill_switch_enabled() -> bool {
    std::env::var("THINCLAW_ROUTING_KILL_SWITCH")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn create_provider_for_runtime_slug(
    provider: &str,
    model: &str,
    base_config: &crate::config::LlmConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    if crate::config::provider_catalog::endpoint_for(provider).is_some() {
        return create_provider_for_catalog_entry(provider, model);
    }

    let mut llm_config = base_config.clone();
    llm_config.backend = match provider {
        "openai" => LlmBackend::OpenAi,
        "anthropic" => LlmBackend::Anthropic,
        "gemini" => LlmBackend::Gemini,
        "tinfoil" => LlmBackend::Tinfoil,
        "ollama" => LlmBackend::Ollama,
        "openai_compatible" | "openrouter" => LlmBackend::OpenAiCompatible,
        "bedrock" => LlmBackend::Bedrock,
        "llama_cpp" => LlmBackend::LlamaCpp,
        other => {
            return Err(LlmError::RequestFailed {
                provider: "runtime".to_string(),
                reason: format!("Unknown provider slug '{}'", other),
            });
        }
    };

    apply_model_override(&mut llm_config, model);
    create_llm_provider(&llm_config)
}

fn apply_model_override(config: &mut crate::config::LlmConfig, model: &str) {
    match config.backend {
        LlmBackend::OpenAi => {
            if let Some(ref mut openai) = config.openai {
                openai.model = model.to_string();
            }
        }
        LlmBackend::Anthropic => {
            if let Some(ref mut anthropic) = config.anthropic {
                anthropic.model = model.to_string();
            }
        }
        LlmBackend::Ollama => {
            if let Some(ref mut ollama) = config.ollama {
                ollama.model = model.to_string();
            }
        }
        LlmBackend::OpenAiCompatible => {
            if let Some(ref mut compat) = config.openai_compatible {
                compat.model = model.to_string();
            }
        }
        LlmBackend::Tinfoil => {
            if let Some(ref mut tinfoil) = config.tinfoil {
                tinfoil.model = model.to_string();
            }
        }
        LlmBackend::Gemini => {
            if let Some(ref mut gemini) = config.gemini {
                gemini.model = model.to_string();
            }
        }
        LlmBackend::Bedrock => {
            if let Some(ref mut bedrock) = config.bedrock {
                bedrock.model_id = model.to_string();
            }
        }
        LlmBackend::LlamaCpp => {
            if let Some(ref mut llama_cpp) = config.llama_cpp {
                llama_cpp.model = model.to_string();
            }
        }
    }
}

fn build_routing_policy(settings: &ProvidersSettings) -> RoutingPolicy {
    let default_target = if settings.routing_mode == RoutingMode::Policy
        && settings.smart_routing_enabled
        && !provider_slot_selectors(settings, ProviderModelRole::Cheap).is_empty()
    {
        "cheap"
    } else {
        "primary"
    };
    let mut policy = RoutingPolicy::new(default_target);
    policy.set_enabled(settings.smart_routing_enabled);
    for rule in &settings.policy_rules {
        policy.add_rule(rule.clone());
    }
    policy
}

fn legacy_primary_slug_from_config(config: &Config) -> Option<String> {
    match config.llm.backend {
        LlmBackend::OpenAi => Some("openai".to_string()),
        LlmBackend::Anthropic => Some("anthropic".to_string()),
        LlmBackend::Ollama => Some("ollama".to_string()),
        LlmBackend::OpenAiCompatible => config
            .llm
            .openai_compatible
            .as_ref()
            .and_then(|compat| {
                if compat.base_url.contains("openrouter.ai") {
                    Some("openrouter".to_string())
                } else {
                    None
                }
            })
            .or_else(|| Some("openai_compatible".to_string())),
        LlmBackend::Tinfoil => Some("tinfoil".to_string()),
        LlmBackend::Gemini => Some("gemini".to_string()),
        LlmBackend::Bedrock => Some("bedrock".to_string()),
        LlmBackend::LlamaCpp => Some("llama_cpp".to_string()),
    }
}

pub fn validate_providers_settings(raw: &ProvidersSettings) -> Vec<String> {
    let mut diagnostics = validate_planner_settings(raw);

    if raw.smart_routing_enabled && raw.enabled.is_empty() {
        diagnostics.push(
            "smart_routing_enabled is true but no providers are enabled; runtime will fall back to legacy backend".to_string(),
        );
    }
    if raw.routing_mode == RoutingMode::CheapSplit
        && raw.cheap_model.is_none()
        && raw.preferred_cheap_provider.is_none()
    {
        diagnostics.push(
            "CheapSplit mode is enabled but cheap model is not explicitly configured; runtime defaults will infer one when possible".to_string(),
        );
    }
    if raw.routing_mode == RoutingMode::Policy {
        for target in policy_rule_targets(&raw.policy_rules) {
            if !route_target_resolves_in_settings(raw, &target) {
                diagnostics.push(format!(
                    "Policy target '{}' cannot be resolved with current provider configuration",
                    target
                ));
            }
        }
    }

    diagnostics.sort();
    diagnostics.dedup();
    diagnostics
}

pub fn derive_runtime_defaults(settings: &Settings) -> ProvidersSettings {
    derive_runtime_defaults_from_parts(
        settings.providers.clone(),
        legacy_primary_slug(settings),
        settings.selected_model.clone(),
    )
}

/// Backward-compatible alias. New code should prefer `derive_runtime_defaults`.
pub fn normalize_providers_settings(settings: &Settings) -> ProvidersSettings {
    derive_runtime_defaults(settings)
}

fn derive_runtime_defaults_from_parts(
    mut providers: ProvidersSettings,
    legacy_primary: Option<String>,
    legacy_model: Option<String>,
) -> ProvidersSettings {
    if providers.primary.is_none() {
        providers.primary = legacy_primary;
    }
    if providers.primary_model.is_none() {
        providers.primary_model = legacy_model;
    }
    if let Some(primary_slug) = providers.primary.clone() {
        let slots = providers.provider_models.entry(primary_slug).or_default();
        if slots.primary.is_none() {
            slots.primary = providers.primary_model.clone();
        }
    }
    if let Some(spec) = providers.cheap_model.clone()
        && let Some((slug, model)) = spec.split_once('/')
    {
        let slots = providers
            .provider_models
            .entry(slug.to_string())
            .or_default();
        if slots.cheap.is_none() {
            slots.cheap = Some(model.to_string());
        }
        if providers.preferred_cheap_provider.is_none() {
            providers.preferred_cheap_provider = Some(slug.to_string());
        }
    }
    for (slug, allowed) in &providers.allowed_models {
        if let Some(model) = allowed.first() {
            let slots = providers.provider_models.entry(slug.clone()).or_default();
            if slots.primary.is_none() {
                slots.primary = Some(model.clone());
            }
        }
    }

    let mut enabled = BTreeSet::new();
    for provider in &providers.enabled {
        enabled.insert(provider.clone());
    }
    if let Some(primary) = providers.primary.as_ref() {
        enabled.insert(primary.clone());
    }
    if let Some((slug, _)) = providers
        .cheap_model
        .as_deref()
        .and_then(|spec| spec.split_once('/'))
    {
        enabled.insert(slug.to_string());
    }
    if let Some(preferred_cheap_provider) = providers.preferred_cheap_provider.as_ref() {
        enabled.insert(preferred_cheap_provider.clone());
    }
    for entry in &providers.fallback_chain {
        if let Some((slug, _)) = entry.split_once('/') {
            enabled.insert(slug.to_string());
        } else if let Some((slug, _)) = parse_provider_slot_selector(entry) {
            enabled.insert(slug.to_string());
        }
    }
    for slug in providers.allowed_models.keys() {
        enabled.insert(slug.clone());
    }
    providers.enabled = enabled.into_iter().collect();

    if providers.primary.is_none() {
        providers.primary = providers
            .primary_pool_order
            .iter()
            .find(|slug| providers.enabled.iter().any(|enabled| enabled == *slug))
            .cloned()
            .or_else(|| providers.enabled.first().cloned());
    }

    let enabled_snapshot = providers.enabled.clone();
    for slug in enabled_snapshot {
        let slots = providers.provider_models.entry(slug.clone()).or_default();

        if slots.primary.is_none() {
            slots.primary = if providers.primary.as_deref() == Some(slug.as_str()) {
                providers.primary_model.clone()
            } else {
                providers
                    .allowed_models
                    .get(&slug)
                    .and_then(|models| models.first().cloned())
            }
            .or_else(|| default_model_for_runtime_slug(&slug).map(ToOwned::to_owned));
        }

        if slots.cheap.is_none() {
            slots.cheap = suggest_provider_cheap_model(&slug, slots.primary.as_deref())
                .or_else(|| slots.primary.clone());
        }
    }

    if let Some(primary_slug) = providers.primary.as_deref() {
        providers.primary_model = providers
            .provider_models
            .get(primary_slug)
            .and_then(|slots| slots.primary.clone())
            .or_else(|| providers.primary_model.clone());
    }

    if providers.preferred_cheap_provider.is_none() {
        providers.preferred_cheap_provider = providers
            .cheap_pool_order
            .iter()
            .find(|slug| {
                providers.enabled.iter().any(|enabled| enabled == *slug)
                    && provider_role_target(&providers, slug, ProviderModelRole::Cheap).is_some()
            })
            .cloned()
            .or_else(|| providers.primary.clone())
            .or_else(|| {
                provider_slot_selectors(&providers, ProviderModelRole::Cheap)
                    .into_iter()
                    .next()
                    .and_then(|selector| {
                        parse_provider_slot_selector(&selector).map(|(slug, _)| slug.to_string())
                    })
            });
    }

    providers.primary_pool_order =
        normalized_provider_pool_order(&providers, ProviderModelRole::Primary);
    providers.cheap_pool_order =
        normalized_provider_pool_order(&providers, ProviderModelRole::Cheap);

    if providers.smart_routing_enabled
        && providers.routing_mode == RoutingMode::PrimaryOnly
        && (!provider_slot_selectors(&providers, ProviderModelRole::Cheap).is_empty()
            || providers.enabled.len() > 1)
    {
        providers.routing_mode = RoutingMode::CheapSplit;
    }

    providers.cheap_model = preferred_cheap_target(&providers).or_else(|| {
        suggest_cheap_model(
            providers.primary.as_deref(),
            providers.primary_model.as_deref(),
            &providers.enabled,
        )
    });

    if providers.fallback_chain.is_empty() {
        providers.fallback_chain = providers
            .enabled
            .iter()
            .filter(|slug| providers.primary.as_deref() != Some(slug.as_str()))
            .map(|slug| provider_slot_selector(slug, ProviderModelRole::Primary))
            .collect();
    }

    if providers.routing_mode == RoutingMode::Policy && providers.policy_rules.is_empty() {
        providers.policy_rules = vec![
            RoutingRule::VisionContent {
                provider: "primary".to_string(),
            },
            RoutingRule::LargeContext {
                threshold: 120_000,
                provider: "primary".to_string(),
            },
        ];
    }

    providers
}

fn legacy_primary_slug(settings: &Settings) -> Option<String> {
    match settings.llm_backend.as_deref() {
        Some("openai") => Some("openai".to_string()),
        Some("anthropic") => Some("anthropic".to_string()),
        Some("ollama") => Some("ollama".to_string()),
        Some("openai_compatible") => settings
            .openai_compatible_base_url
            .as_deref()
            .and_then(|url| {
                if url.contains("openrouter.ai") {
                    Some("openrouter".to_string())
                } else {
                    None
                }
            })
            .or_else(|| Some("openai_compatible".to_string())),
        Some("tinfoil") => Some("tinfoil".to_string()),
        Some("gemini") => Some("gemini".to_string()),
        Some("bedrock") => Some("bedrock".to_string()),
        Some("llama_cpp") => Some("llama_cpp".to_string()),
        _ => None,
    }
}

fn suggest_cheap_model(
    primary: Option<&str>,
    primary_model: Option<&str>,
    enabled: &[String],
) -> Option<String> {
    if let Some(primary_slug) = primary
        && let Some(candidate_model) = suggest_provider_cheap_model(primary_slug, primary_model)
        && primary_model != Some(candidate_model.as_str())
    {
        let candidate = format!("{primary_slug}/{candidate_model}");
        return Some(candidate);
    }

    enabled
        .iter()
        .filter(|slug| Some(slug.as_str()) != primary)
        .filter_map(|slug| {
            suggest_provider_cheap_model(slug, None)
                .or_else(|| default_model_for_runtime_slug(slug).map(ToOwned::to_owned))
                .map(|model| format!("{slug}/{model}"))
        })
        .min_by(|a, b| {
            route_target_known_cost_per_m_usd(a)
                .partial_cmp(&route_target_known_cost_per_m_usd(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn suggest_provider_cheap_model(slug: &str, primary_model: Option<&str>) -> Option<String> {
    let mapped = match slug {
        "openai" => Some("gpt-4o-mini"),
        "anthropic" => Some("claude-sonnet-4-6"),
        "gemini" => Some("gemini-2.5-flash-lite"),
        "openrouter" => Some("openai/gpt-4o-mini"),
        "tinfoil" => Some("kimi-k2-5"),
        _ => None,
    };

    if let Some(candidate) = mapped
        && primary_model != Some(candidate)
    {
        return Some(candidate.to_string());
    }

    default_model_for_runtime_slug(slug)
        .map(ToOwned::to_owned)
        .filter(|model| primary_model != Some(model.as_str()))
}

fn default_model_for_runtime_slug(slug: &str) -> Option<&'static str> {
    crate::config::provider_catalog::endpoint_for(slug)
        .map(|endpoint| endpoint.default_model.as_str())
        .or(match slug {
            "ollama" => Some("llama3"),
            "openai_compatible" => Some("default"),
            "bedrock" => Some("anthropic.claude-3-sonnet-20240229-v1:0"),
            "llama_cpp" => Some("llama-local"),
            _ => None,
        })
}

fn provider_slot_selector(slug: &str, role: ProviderModelRole) -> String {
    format!("{slug}@{}", role.as_str())
}

fn parse_provider_slot_selector(selector: &str) -> Option<(&str, ProviderModelRole)> {
    if let Some(slug) = selector.strip_suffix("@primary") {
        return Some((slug, ProviderModelRole::Primary));
    }
    if let Some(slug) = selector.strip_suffix("@cheap") {
        return Some((slug, ProviderModelRole::Cheap));
    }
    None
}

fn policy_rule_targets(rules: &[RoutingRule]) -> Vec<String> {
    let mut targets = Vec::new();
    let mut push_unique = |target: &str| {
        if !targets.iter().any(|existing| existing == target) {
            targets.push(target.to_string());
        }
    };

    for rule in rules {
        match rule {
            RoutingRule::LargeContext { provider, .. }
            | RoutingRule::VisionContent { provider } => {
                push_unique(provider);
            }
            RoutingRule::RoundRobin { providers } => {
                for provider in providers {
                    push_unique(provider);
                }
            }
            RoutingRule::Fallback { primary, fallbacks } => {
                push_unique(primary);
                for fallback in fallbacks {
                    push_unique(fallback);
                }
            }
            RoutingRule::CostOptimized { .. } | RoutingRule::LowestLatency => {}
        }
    }

    targets
}

fn route_target_resolves_in_settings(settings: &ProvidersSettings, target: &str) -> bool {
    match target {
        "primary" => settings.primary.is_some() || !settings.enabled.is_empty(),
        "cheap" => {
            settings.cheap_model.is_some()
                || settings.preferred_cheap_provider.is_some()
                || settings.primary.is_some()
                || !settings.enabled.is_empty()
        }
        other if parse_provider_slot_selector(other).is_some() => {
            let (slug, role) = parse_provider_slot_selector(other)
                .expect("slot selector checked above for route_target_resolves_in_settings");
            provider_declared_for_routing(settings, slug)
                && provider_role_target(settings, slug, role).is_some()
        }
        other => {
            if let Some((slug, _)) = other.split_once('/') {
                provider_declared_for_routing(settings, slug)
            } else {
                false
            }
        }
    }
}

fn provider_declared_for_routing(settings: &ProvidersSettings, slug: &str) -> bool {
    if settings.enabled.iter().any(|entry| entry == slug)
        || settings.primary.as_deref() == Some(slug)
        || settings.preferred_cheap_provider.as_deref() == Some(slug)
        || settings.allowed_models.contains_key(slug)
    {
        return true;
    }

    if settings
        .cheap_model
        .as_deref()
        .and_then(|spec| spec.split_once('/'))
        .is_some_and(|(cheap_slug, _)| cheap_slug == slug)
    {
        return true;
    }

    settings.fallback_chain.iter().any(|target| {
        target
            .split_once('/')
            .map(|(fallback_slug, _)| fallback_slug == slug)
            .or_else(|| {
                parse_provider_slot_selector(target).map(|(fallback_slug, _)| fallback_slug == slug)
            })
            .unwrap_or(false)
    })
}

fn provider_role_target(
    settings: &ProvidersSettings,
    slug: &str,
    role: ProviderModelRole,
) -> Option<String> {
    let slots = settings.provider_models.get(slug);
    let model = match role {
        ProviderModelRole::Primary => slots
            .and_then(|entry| entry.primary.clone())
            .or_else(|| {
                if settings.primary.as_deref() == Some(slug) {
                    settings.primary_model.clone()
                } else {
                    settings
                        .allowed_models
                        .get(slug)
                        .and_then(|models| models.first().cloned())
                }
            })
            .or_else(|| default_model_for_runtime_slug(slug).map(ToOwned::to_owned)),
        ProviderModelRole::Cheap => slots
            .and_then(|entry| entry.cheap.clone())
            .or_else(|| {
                settings
                    .cheap_model
                    .as_deref()
                    .and_then(|spec| spec.split_once('/'))
                    .and_then(|(cheap_slug, model)| {
                        if cheap_slug == slug {
                            Some(model.to_string())
                        } else {
                            None
                        }
                    })
            })
            .or_else(|| {
                provider_role_target(settings, slug, ProviderModelRole::Primary)
                    .and_then(|target| target.split_once('/').map(|(_, model)| model.to_string()))
            })
            .or_else(|| {
                suggest_provider_cheap_model(
                    slug,
                    provider_role_target(settings, slug, ProviderModelRole::Primary)
                        .as_deref()
                        .and_then(|target| target.split_once('/').map(|(_, model)| model)),
                )
            }),
    }?;

    Some(format!("{slug}/{model}"))
}

fn provider_slot_selectors(settings: &ProvidersSettings, role: ProviderModelRole) -> Vec<String> {
    let mut selectors = Vec::new();
    let push = |slug: &str, selectors: &mut Vec<String>| {
        if provider_role_target(settings, slug, role).is_some() {
            let selector = provider_slot_selector(slug, role);
            if !selectors.contains(&selector) {
                selectors.push(selector);
            }
        }
    };

    match role {
        ProviderModelRole::Primary => {
            for slug in &settings.primary_pool_order {
                push(slug, &mut selectors);
            }
            for slug in &settings.enabled {
                if !settings
                    .primary_pool_order
                    .iter()
                    .any(|ordered| ordered == slug)
                {
                    push(slug, &mut selectors);
                }
            }
        }
        ProviderModelRole::Cheap => {
            for slug in &settings.cheap_pool_order {
                push(slug, &mut selectors);
            }
            let mut remaining: Vec<(String, String)> = settings
                .enabled
                .iter()
                .filter(|slug| {
                    !settings
                        .cheap_pool_order
                        .iter()
                        .any(|ordered| ordered == *slug)
                })
                .filter_map(|slug| {
                    provider_role_target(settings, slug, role).map(|target| (slug.clone(), target))
                })
                .collect();
            remaining.sort_by(|a, b| {
                route_target_known_cost_per_m_usd(&a.1)
                    .partial_cmp(&route_target_known_cost_per_m_usd(&b.1))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for (slug, _) in remaining {
                push(&slug, &mut selectors);
            }
        }
    }

    selectors
}

fn normalized_provider_pool_order(
    settings: &ProvidersSettings,
    role: ProviderModelRole,
) -> Vec<String> {
    let mut order = Vec::new();
    let push = |slug: &str, order: &mut Vec<String>| {
        if settings.enabled.iter().any(|enabled| enabled == slug)
            && provider_role_target(settings, slug, role).is_some()
            && !order.iter().any(|existing| existing == slug)
        {
            order.push(slug.to_string());
        }
    };

    match role {
        ProviderModelRole::Primary => {
            if let Some(primary_slug) = settings.primary.as_deref() {
                push(primary_slug, &mut order);
            }
            for slug in &settings.primary_pool_order {
                push(slug, &mut order);
            }
            for slug in &settings.enabled {
                push(slug, &mut order);
            }
        }
        ProviderModelRole::Cheap => {
            if let Some(preferred_slug) = settings.preferred_cheap_provider.as_deref() {
                push(preferred_slug, &mut order);
            }
            for slug in &settings.cheap_pool_order {
                push(slug, &mut order);
            }
            let mut remaining: Vec<(String, String)> = settings
                .enabled
                .iter()
                .filter(|slug| !order.iter().any(|existing| existing == *slug))
                .filter_map(|slug| {
                    provider_role_target(settings, slug, role).map(|target| (slug.clone(), target))
                })
                .collect();
            remaining.sort_by(|a, b| {
                route_target_known_cost_per_m_usd(&a.1)
                    .partial_cmp(&route_target_known_cost_per_m_usd(&b.1))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for (slug, _) in remaining {
                push(&slug, &mut order);
            }
        }
    }

    order
}

fn preferred_cheap_target(settings: &ProvidersSettings) -> Option<String> {
    if let Some(slug) = settings.preferred_cheap_provider.as_deref() {
        return provider_role_target(settings, slug, ProviderModelRole::Cheap);
    }
    provider_slot_selectors(settings, ProviderModelRole::Cheap)
        .into_iter()
        .next()
        .and_then(|selector| {
            parse_provider_slot_selector(&selector).map(|(slug, role)| (slug.to_string(), role))
        })
        .and_then(|(slug, role)| provider_role_target(settings, &slug, role))
}

fn route_target_known_cost_per_m_usd(target: &str) -> f64 {
    if matches!(target, "primary" | "cheap") {
        let (input, output) = crate::llm::costs::default_cost();
        return ((input + output) * rust_decimal::Decimal::from(1_000_000u64))
            .to_f64()
            .unwrap_or(f64::MAX);
    }

    if let Some((slug, role)) = parse_provider_slot_selector(target)
        && let Some(concrete_target) = provider_role_target(
            &ProvidersSettings {
                enabled: vec![slug.to_string()],
                primary: Some(slug.to_string()),
                preferred_cheap_provider: Some(slug.to_string()),
                ..ProvidersSettings::default()
            },
            slug,
            role,
        )
    {
        return route_target_known_cost_per_m_usd(&concrete_target);
    }

    let (input, output) = crate::llm::costs::model_cost(target).unwrap_or_else(|| {
        if target.starts_with("ollama/") || target.starts_with("llama_cpp/") {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        } else {
            crate::llm::costs::default_cost()
        }
    });
    ((input + output) * rust_decimal::Decimal::from(1_000_000u64))
        .to_f64()
        .unwrap_or(f64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::settings::ProviderModelSlots;

    #[test]
    fn resolved_completion_request_clears_model_override() {
        let request = CompletionRequest::new(vec![crate::llm::ChatMessage::user("hi")])
            .with_model("openai/gpt-5.4-mini");

        let resolved = RuntimeLlmProvider::resolved_completion_request(
            request,
            "test|openai|gpt-5.4-mini",
            "test",
        );

        assert!(resolved.model.is_none());
    }

    #[test]
    fn resolved_tool_completion_request_clears_model_override() {
        let request =
            ToolCompletionRequest::new(vec![crate::llm::ChatMessage::user("hi")], Vec::new())
                .with_model("openai/gpt-5.4-mini");

        let resolved = RuntimeLlmProvider::resolved_tool_completion_request(
            request,
            "test|openai|gpt-5.4-mini",
            "test",
        );

        assert!(resolved.model.is_none());
    }

    #[test]
    fn normalize_promotes_legacy_models_into_provider_slots() {
        let mut settings = Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4o".to_string()),
            ..Settings::default()
        };
        settings.providers.cheap_model = Some("openai/gpt-4o-mini".to_string());

        let providers = normalize_providers_settings(&settings);
        let openai = providers
            .provider_models
            .get("openai")
            .expect("openai slots should exist");

        assert_eq!(providers.primary.as_deref(), Some("openai"));
        assert_eq!(providers.primary_model.as_deref(), Some("gpt-4o"));
        assert_eq!(
            providers.preferred_cheap_provider.as_deref(),
            Some("openai")
        );
        assert_eq!(openai.primary.as_deref(), Some("gpt-4o"));
        assert_eq!(openai.cheap.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn provider_slot_selectors_prioritize_primary_and_preferred_cheap() {
        let mut providers = ProvidersSettings {
            enabled: vec![
                "anthropic".to_string(),
                "openai".to_string(),
                "gemini".to_string(),
            ],
            primary: Some("anthropic".to_string()),
            preferred_cheap_provider: Some("gemini".to_string()),
            ..ProvidersSettings::default()
        };
        providers.provider_models.insert(
            "anthropic".to_string(),
            ProviderModelSlots {
                primary: Some("claude-opus-4-7".to_string()),
                cheap: Some("claude-sonnet-4-6".to_string()),
            },
        );
        providers.provider_models.insert(
            "openai".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-4o".to_string()),
                cheap: Some("gpt-4o-mini".to_string()),
            },
        );
        providers.provider_models.insert(
            "gemini".to_string(),
            ProviderModelSlots {
                primary: Some("gemini-2.5-flash".to_string()),
                cheap: Some("gemini-2.5-flash-lite".to_string()),
            },
        );

        let primary_targets = provider_slot_selectors(&providers, ProviderModelRole::Primary);
        let cheap_targets = provider_slot_selectors(&providers, ProviderModelRole::Cheap);

        assert_eq!(
            primary_targets.first().map(String::as_str),
            Some("anthropic@primary")
        );
        assert_eq!(
            cheap_targets.first().map(String::as_str),
            Some("gemini@cheap")
        );
        assert!(cheap_targets.iter().any(|target| target == "openai@cheap"));
    }

    #[test]
    fn provider_slot_selectors_respect_explicit_pool_order() {
        let mut providers = ProvidersSettings {
            enabled: vec![
                "anthropic".to_string(),
                "openai".to_string(),
                "gemini".to_string(),
            ],
            primary: Some("anthropic".to_string()),
            preferred_cheap_provider: Some("gemini".to_string()),
            primary_pool_order: vec![
                "openai".to_string(),
                "anthropic".to_string(),
                "gemini".to_string(),
            ],
            cheap_pool_order: vec![
                "openai".to_string(),
                "gemini".to_string(),
                "anthropic".to_string(),
            ],
            ..ProvidersSettings::default()
        };
        providers.provider_models.insert(
            "anthropic".to_string(),
            ProviderModelSlots {
                primary: Some("claude-opus-4-7".to_string()),
                cheap: Some("claude-sonnet-4-6".to_string()),
            },
        );
        providers.provider_models.insert(
            "openai".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-4o".to_string()),
                cheap: Some("gpt-4o-mini".to_string()),
            },
        );
        providers.provider_models.insert(
            "gemini".to_string(),
            ProviderModelSlots {
                primary: Some("gemini-2.5-flash".to_string()),
                cheap: Some("gemini-2.5-flash-lite".to_string()),
            },
        );

        let primary_targets = provider_slot_selectors(&providers, ProviderModelRole::Primary);
        let cheap_targets = provider_slot_selectors(&providers, ProviderModelRole::Cheap);

        assert_eq!(
            primary_targets,
            vec![
                "openai@primary".to_string(),
                "anthropic@primary".to_string(),
                "gemini@primary".to_string(),
            ]
        );
        assert_eq!(
            cheap_targets,
            vec![
                "openai@cheap".to_string(),
                "gemini@cheap".to_string(),
                "anthropic@cheap".to_string(),
            ]
        );
    }

    #[test]
    fn normalize_populates_pool_orders_from_role_preferences() {
        let mut settings = Settings::default();
        settings.providers.enabled = vec!["openai".to_string(), "anthropic".to_string()];
        settings.providers.primary = Some("anthropic".to_string());
        settings.providers.preferred_cheap_provider = Some("openai".to_string());
        settings.providers.provider_models.insert(
            "openai".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-4o".to_string()),
                cheap: Some("gpt-4o-mini".to_string()),
            },
        );
        settings.providers.provider_models.insert(
            "anthropic".to_string(),
            ProviderModelSlots {
                primary: Some("claude-opus-4-7".to_string()),
                cheap: Some("claude-sonnet-4-6".to_string()),
            },
        );

        let providers = normalize_providers_settings(&settings);

        assert_eq!(
            providers.primary_pool_order,
            vec!["anthropic".to_string(), "openai".to_string()]
        );
        assert_eq!(
            providers.cheap_pool_order,
            vec!["openai".to_string(), "anthropic".to_string()]
        );
    }

    #[test]
    fn provider_models_do_not_auto_enable_disabled_providers() {
        let mut settings = Settings::default();
        settings.providers.provider_models.insert(
            "openai".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-4o".to_string()),
                cheap: Some("gpt-4o-mini".to_string()),
            },
        );

        let providers = normalize_providers_settings(&settings);

        assert!(providers.enabled.is_empty());
        assert_eq!(
            providers
                .provider_models
                .get("openai")
                .and_then(|slots| slots.primary.as_deref()),
            Some("gpt-4o")
        );
        assert_eq!(
            providers
                .provider_models
                .get("openai")
                .and_then(|slots| slots.cheap.as_deref()),
            Some("gpt-4o-mini")
        );
    }

    #[test]
    fn policy_rule_targets_collect_direct_and_fallback_targets() {
        let rules = vec![
            RoutingRule::LargeContext {
                threshold: 8_000,
                provider: "openai/gpt-4o".to_string(),
            },
            RoutingRule::RoundRobin {
                providers: vec!["openai@cheap".to_string(), "anthropic@primary".to_string()],
            },
            RoutingRule::Fallback {
                primary: "primary".to_string(),
                fallbacks: vec!["openai/gpt-4o-mini".to_string()],
            },
        ];

        let targets = policy_rule_targets(&rules);
        assert!(targets.iter().any(|target| target == "openai/gpt-4o"));
        assert!(targets.iter().any(|target| target == "openai@cheap"));
        assert!(targets.iter().any(|target| target == "anthropic@primary"));
        assert!(targets.iter().any(|target| target == "openai/gpt-4o-mini"));
    }

    #[test]
    fn validate_flags_unresolvable_policy_targets() {
        let mut providers = ProvidersSettings {
            enabled: vec!["openai".to_string()],
            routing_mode: RoutingMode::Policy,
            ..ProvidersSettings::default()
        };
        providers.policy_rules = vec![RoutingRule::VisionContent {
            provider: "anthropic@primary".to_string(),
        }];

        let diagnostics = validate_providers_settings(&providers);
        assert!(
            diagnostics
                .iter()
                .any(|entry| entry.contains("cannot be resolved")),
            "expected unresolved policy target diagnostic, got: {:?}",
            diagnostics
        );
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn advisor_target_primary_resolves_to_primary_provider_in_advisor_executor() {
        let config = {
            let _env_guard = crate::config::helpers::lock_env();
            // SAFETY: guarded by crate-wide ENV_MUTEX.
            unsafe {
                std::env::set_var("LLM_BACKEND", "openai_compatible");
                std::env::set_var("LLM_BASE_URL", "http://localhost:12345/v1");
                std::env::set_var("LLM_MODEL", "gpt-5.4");
                std::env::set_var(
                    "SECRETS_MASTER_KEY",
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                );
            }
            let loaded = Config::from_env().await.expect("config should load");
            // SAFETY: guarded by crate-wide ENV_MUTEX.
            unsafe {
                std::env::remove_var("LLM_BACKEND");
                std::env::remove_var("LLM_BASE_URL");
                std::env::remove_var("LLM_MODEL");
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
            loaded
        };

        let mut providers = ProvidersSettings {
            enabled: vec!["openai_compatible".to_string()],
            primary: Some("openai_compatible".to_string()),
            primary_model: Some("gpt-5.4".to_string()),
            cheap_model: Some("openai_compatible/gpt-5.4-mini".to_string()),
            smart_routing_enabled: true,
            routing_mode: RoutingMode::AdvisorExecutor,
            ..ProvidersSettings::default()
        };
        providers.provider_models.insert(
            "openai_compatible".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-5.4".to_string()),
                cheap: Some("gpt-5.4-mini".to_string()),
            },
        );

        let manager = LlmRuntimeManager::new(config, providers, None, None, "test-user", None)
            .expect("runtime manager should build");

        let provider = manager
            .provider_handle_for_target("primary")
            .expect("primary advisor target should resolve");

        assert_eq!(provider.active_model_name(), "gpt-5.4");
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn advisor_executor_status_reports_readiness_and_targets() {
        let config = {
            let _env_guard = crate::config::helpers::lock_env();
            unsafe {
                std::env::set_var("LLM_BACKEND", "openai_compatible");
                std::env::set_var("LLM_BASE_URL", "http://localhost:12345/v1");
                std::env::set_var("LLM_MODEL", "gpt-5.4");
                std::env::set_var(
                    "SECRETS_MASTER_KEY",
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                );
            }
            let loaded = Config::from_env().await.expect("config should load");
            unsafe {
                std::env::remove_var("LLM_BACKEND");
                std::env::remove_var("LLM_BASE_URL");
                std::env::remove_var("LLM_MODEL");
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
            loaded
        };

        let mut providers = ProvidersSettings {
            enabled: vec!["openai_compatible".to_string()],
            primary: Some("openai_compatible".to_string()),
            primary_model: Some("gpt-5.4".to_string()),
            cheap_model: Some("openai_compatible/gpt-5.4-mini".to_string()),
            smart_routing_enabled: true,
            routing_mode: RoutingMode::AdvisorExecutor,
            ..ProvidersSettings::default()
        };
        providers.provider_models.insert(
            "openai_compatible".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-5.4".to_string()),
                cheap: Some("gpt-5.4-mini".to_string()),
            },
        );

        let manager = LlmRuntimeManager::new(config, providers, None, None, "test-user", None)
            .expect("runtime manager should build");
        let status = manager.status();

        assert!(status.advisor_ready);
        assert_eq!(
            status.executor_target.as_deref(),
            Some("openai_compatible/gpt-5.4-mini")
        );
        assert_eq!(
            status.advisor_target.as_deref(),
            Some("openai_compatible/gpt-5.4")
        );
        assert_eq!(
            status.advisor_auto_escalation_mode,
            crate::settings::AdvisorAutoEscalationMode::RiskAndComplexFinal
        );
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn advisor_ready_callback_reports_current_readiness_immediately() {
        let config = {
            let _env_guard = crate::config::helpers::lock_env();
            unsafe {
                std::env::set_var("LLM_BACKEND", "openai_compatible");
                std::env::set_var("LLM_BASE_URL", "http://localhost:12345/v1");
                std::env::set_var("LLM_MODEL", "gpt-5.4");
                std::env::set_var(
                    "SECRETS_MASTER_KEY",
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                );
            }
            let loaded = Config::from_env().await.expect("config should load");
            unsafe {
                std::env::remove_var("LLM_BACKEND");
                std::env::remove_var("LLM_BASE_URL");
                std::env::remove_var("LLM_MODEL");
                std::env::remove_var("SECRETS_MASTER_KEY");
            }
            loaded
        };

        let mut providers = ProvidersSettings {
            enabled: vec!["openai_compatible".to_string()],
            primary: Some("openai_compatible".to_string()),
            primary_model: Some("gpt-5.4".to_string()),
            cheap_model: Some("openai_compatible/gpt-5.4-mini".to_string()),
            smart_routing_enabled: true,
            routing_mode: RoutingMode::AdvisorExecutor,
            ..ProvidersSettings::default()
        };
        providers.provider_models.insert(
            "openai_compatible".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-5.4".to_string()),
                cheap: Some("gpt-5.4-mini".to_string()),
            },
        );

        let manager = LlmRuntimeManager::new(config, providers, None, None, "test-user", None)
            .expect("runtime manager should build");
        let seen = Arc::new(std::sync::Mutex::new(None));
        let seen_for_callback = Arc::clone(&seen);

        manager.set_advisor_ready_callback(move |advisor_ready| {
            *seen_for_callback.lock().expect("callback lock") = Some(advisor_ready);
        });

        assert_eq!(*seen.lock().expect("callback lock"), Some(true));
    }
}
