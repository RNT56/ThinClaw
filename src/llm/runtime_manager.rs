use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

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
use crate::llm::routing_policy::{
    RouteCandidate, RoutingContext, RoutingDecision, RoutingPolicy, RoutingRule,
};
use crate::llm::route_planner::{
    RequiredCapabilities, RoutePlanner, RoutePlannerInput,
    validate_providers_settings as validate_planner_settings,
};
use crate::llm::smart_routing::{SmartRoutingConfig, TaskComplexity, classify_message};
use crate::llm::{CooldownConfig, FailoverProvider, RetryConfig, RetryProvider};
use crate::secrets::SecretsStore;
use crate::settings::{ProviderModelSlots, ProvidersSettings, RoutingMode, Settings};

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
}

#[derive(Clone)]
struct LlmRuntimeSnapshot {
    config: Config,
    providers: ProvidersSettings,
    llm: Arc<dyn LlmProvider>,
    cheap_llm: Option<Arc<dyn LlmProvider>>,
}

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
    ) -> RoutingContext {
        let estimated_input_tokens = messages
            .iter()
            .map(|m| (m.estimated_chars() / 4) as u32)
            .sum();
        let has_vision = messages.iter().any(|m| {
            m.attachments
                .iter()
                .any(|a| a.mime_type.starts_with("image/"))
        });

        RoutingContext {
            estimated_input_tokens,
            has_vision,
            has_tools,
            requires_streaming,
            budget_usd: None,
        }
    }

    fn provider_for_request(
        &self,
        requested_model: Option<&str>,
        routing_context: Option<&RoutingContext>,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        let snapshot = self.manager.snapshot();

        if let Some(model) = requested_model {
            return self.manager.provider_for_model_spec(model, &snapshot);
        }

        // ── Shadow mode: run route planner in parallel for diff logging ──
        if snapshot.providers.smart_routing_enabled {
            if let Some(ctx) = routing_context {
                let planner_input = RoutePlannerInput {
                    required_capabilities: RequiredCapabilities::from_routing_context(ctx),
                    routing_mode: snapshot.providers.routing_mode,
                    routing_context: ctx.clone(),
                    model_override: None,
                    provider_health: std::collections::HashMap::new(),
                    candidates: self.manager.available_route_candidates(&snapshot),
                    turn_cost_usd: 0.0,
                    budget_utilization: None,
                    last_user_message: None,
                };

                if let Ok(guard) = self.manager.route_planner.read() {
                    let planner_decision = guard.plan(
                        &planner_input,
                        self.manager.routing_policy.read().ok().as_deref(),
                    );
                    let live_target = self.live_routing_target(&snapshot, ctx);
                    if planner_decision.target != live_target {
                        tracing::info!(
                            live = %live_target,
                            planner = %planner_decision.target,
                            reason = %planner_decision.reason,
                            mode = %snapshot.providers.routing_mode.as_str(),
                            "[route_planner shadow] Decision differs from live path"
                        );
                    }
                }
            }
        }

        // ── Live path (unchanged existing logic) ──
        if matches!(self.role, RuntimeProviderRole::Primary)
            && snapshot.providers.smart_routing_enabled
            && snapshot.providers.routing_mode == RoutingMode::Policy
            && !snapshot.providers.policy_rules.is_empty()
            && let Some(ctx) = routing_context
        {
            let decision = self.manager.route_decision_for_policy(ctx, &snapshot);
            return self
                .manager
                .provider_for_routing_decision(&decision, &snapshot);
        }

        Ok(match self.role {
            RuntimeProviderRole::Primary => snapshot.llm,
            RuntimeProviderRole::Cheap => snapshot.cheap_llm.unwrap_or(snapshot.llm),
        })
    }

    /// Determine the target the live (pre-planner) routing logic would pick.
    /// Used only by shadow mode for diff logging.
    fn live_routing_target(
        &self,
        snapshot: &LlmRuntimeSnapshot,
        ctx: &RoutingContext,
    ) -> String {
        match snapshot.providers.routing_mode {
            RoutingMode::PrimaryOnly => "primary".to_string(),
            RoutingMode::CheapSplit => {
                // CheapSplit in live mode goes through SmartRoutingProvider
                // which wraps primary; tools/streaming → primary is built-in
                if ctx.has_tools || ctx.requires_streaming {
                    "primary".to_string()
                } else {
                    "cheap".to_string() // simplified — actual classification in SmartRouting
                }
            }
            RoutingMode::AdvisorExecutor => {
                // New mode — no existing live path, treat as cheap
                if snapshot.cheap_llm.is_some() {
                    "cheap".to_string()
                } else {
                    "primary".to_string()
                }
            }
            RoutingMode::Policy => {
                let candidates = self.manager.available_route_candidates(snapshot);
                self.manager
                    .routing_policy
                    .read()
                    .ok()
                    .map(|policy| policy.select_decision(ctx, &candidates).target)
                    .unwrap_or_else(|| "primary".to_string())
            }
        }
    }

    fn resolved_completion_request(mut request: CompletionRequest) -> CompletionRequest {
        // The runtime resolves `request.model` to a concrete provider before
        // delegating, so downstream adapters should not see a stale override.
        request.model = None;
        request
    }

    fn resolved_tool_completion_request(
        mut request: ToolCompletionRequest,
    ) -> ToolCompletionRequest {
        // The runtime resolves `request.model` to a concrete provider before
        // delegating, so downstream adapters should not see a stale override.
        request.model = None;
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
        let ctx = self.routing_context(&request.messages, false, false);
        let provider = self.provider_for_request(request.model.as_deref(), Some(&ctx))?;
        provider
            .complete(Self::resolved_completion_request(request))
            .await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let ctx = self.routing_context(&request.messages, true, false);
        let provider = self.provider_for_request(request.model.as_deref(), Some(&ctx))?;
        provider
            .complete_with_tools(Self::resolved_tool_completion_request(request))
            .await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let ctx = self.routing_context(&request.messages, false, true);
        let provider = self.provider_for_request(request.model.as_deref(), Some(&ctx))?;
        provider
            .complete_stream(Self::resolved_completion_request(request))
            .await
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let ctx = self.routing_context(&request.messages, true, true);
        let provider = self.provider_for_request(request.model.as_deref(), Some(&ctx))?;
        provider
            .complete_stream_with_tools(Self::resolved_tool_completion_request(request))
            .await
    }

    fn supports_streaming(&self) -> bool {
        self.current_provider().supports_streaming()
    }

    fn supports_streaming_for_model(&self, requested_model: Option<&str>) -> bool {
        let snapshot = self.manager.snapshot();
        self.provider_for_request(requested_model, None)
            .map(|provider| provider.supports_streaming())
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
    revision: AtomicU64,
    last_error: RwLock<Option<String>>,
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
        let providers = normalize_providers_settings_from_parts(
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

        Ok(Arc::new(Self {
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
            revision: AtomicU64::new(1),
            last_error: RwLock::new(None),
        }))
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
        // Emit planner config validation warnings on reload
        for warning in validate_planner_settings(&providers) {
            tracing::warn!("[route_planner] Config: {}", warning);
        }
        if let Ok(mut last_error) = self.last_error.write() {
            *last_error = None;
        }
        self.revision.fetch_add(1, Ordering::Relaxed);
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
        match target {
            "primary" => {
                if let Some(target) =
                    provider_slot_selectors(&snapshot.providers, ProviderModelRole::Primary)
                        .into_iter()
                        .next()
                {
                    return self.direct_provider_for_route_target(&target, snapshot);
                }
                create_llm_provider(&snapshot.config.llm)
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

    fn provider_for_routing_decision(
        &self,
        decision: &RoutingDecision,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        let mut targets = vec![decision.target.clone()];
        for fallback in &decision.fallbacks {
            if !targets.contains(fallback) {
                targets.push(fallback.clone());
            }
        }
        self.provider_chain_for_targets(&targets, snapshot)
    }

    fn provider_chain_for_targets(
        &self,
        targets: &[String],
        snapshot: &LlmRuntimeSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
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
            return Ok(providers.remove(0));
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

        if rel.max_retries > 0 {
            Ok(Arc::new(RetryProvider::new(
                provider,
                RetryConfig {
                    max_retries: rel.max_retries,
                },
            )))
        } else {
            Ok(provider)
        }
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
            return create_provider_for_runtime_slug(provider, model, &snapshot.config.llm);
        }

        if let Some(primary_provider) = snapshot.providers.primary.as_deref() {
            return create_provider_for_runtime_slug(primary_provider, spec, &snapshot.config.llm);
        }

        let mut llm_config = snapshot.config.llm.clone();
        apply_model_override(&mut llm_config, spec);
        create_llm_provider(&llm_config)
    }

    pub fn simulate_route(&self, ctx: RoutingContext) -> (String, String) {
        self.simulate_route_for_prompt(ctx, None)
    }

    pub fn simulate_route_for_prompt(
        &self,
        ctx: RoutingContext,
        prompt: Option<&str>,
    ) -> (String, String) {
        let snapshot = self.snapshot();
        if !snapshot.providers.smart_routing_enabled {
            return ("primary".to_string(), "Routing disabled".to_string());
        }
        match snapshot.providers.routing_mode {
            RoutingMode::PrimaryOnly => ("primary".to_string(), "Primary-only mode".to_string()),
            RoutingMode::CheapSplit => simulate_cheap_split_route(&snapshot, &ctx, prompt),
            RoutingMode::AdvisorExecutor => {
                if snapshot.cheap_llm.is_some() {
                    ("cheap".to_string(), "AdvisorExecutor: executor handles request, may consult advisor".to_string())
                } else {
                    ("primary".to_string(), "AdvisorExecutor: no executor model, using primary".to_string())
                }
            }
            RoutingMode::Policy => {
                let candidates = self.available_route_candidates(&snapshot);
                let (decision, reason) = self
                    .routing_policy
                    .read()
                    .ok()
                    .map(|policy| {
                        let decision = policy.select_decision(&ctx, &candidates);
                        let reason = decision
                            .matched_rule_index
                            .and_then(|index| {
                                policy
                                    .rules()
                                    .get(index)
                                    .and_then(|rule| {
                                        policy.matches_rule_for_summary(rule, &ctx, &candidates)
                                    })
                                    .map(|_| {
                                        crate::llm::routing_policy::RoutingRuleSummary::from_policy(
                                            &policy,
                                        )
                                        .get(index)
                                        .map(|summary| summary.description.clone())
                                        .unwrap_or_else(|| "Matched policy rule".to_string())
                                    })
                            })
                            .unwrap_or_else(|| "Default policy target".to_string());
                        (decision, reason)
                    })
                    .unwrap_or((
                        RoutingDecision {
                            target: "primary".to_string(),
                            fallbacks: Vec::new(),
                            matched_rule_index: None,
                        },
                        "Default policy target".to_string(),
                    ));

                let reason = if decision.fallbacks.is_empty() {
                    reason
                } else {
                    format!("{}. Fallbacks: {}", reason, decision.fallbacks.join(" -> "))
                };
                (decision.target, reason)
            }
        }
    }

    fn route_decision_for_policy(
        &self,
        ctx: &RoutingContext,
        snapshot: &LlmRuntimeSnapshot,
    ) -> RoutingDecision {
        let candidates = self.available_route_candidates(snapshot);
        self.routing_policy
            .read()
            .ok()
            .map(|policy| policy.select_decision(ctx, &candidates))
            .unwrap_or(RoutingDecision {
                target: "primary".to_string(),
                fallbacks: Vec::new(),
                matched_rule_index: None,
            })
    }

    fn available_route_candidates(&self, snapshot: &LlmRuntimeSnapshot) -> Vec<RouteCandidate> {
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        let mut push = |target: String, cost_per_m_usd: Option<f64>| {
            if seen.insert(target.clone()) {
                candidates.push(RouteCandidate::new(target, cost_per_m_usd));
            }
        };

        push(
            "primary".to_string(),
            self.route_target_cost_per_m_usd("primary", snapshot),
        );

        if !provider_slot_selectors(&snapshot.providers, ProviderModelRole::Cheap).is_empty()
            || snapshot.providers.cheap_model.is_some()
            || snapshot.cheap_llm.is_some()
        {
            push(
                "cheap".to_string(),
                self.route_target_cost_per_m_usd("cheap", snapshot),
            );
        }

        for target in &snapshot.providers.fallback_chain {
            push(
                target.clone(),
                self.route_target_cost_per_m_usd(target, snapshot),
            );
        }

        for slug in &snapshot.providers.enabled {
            let primary_selector = provider_slot_selector(slug, ProviderModelRole::Primary);
            if provider_role_target(&snapshot.providers, slug, ProviderModelRole::Primary).is_some()
            {
                push(
                    primary_selector.clone(),
                    self.route_target_cost_per_m_usd(&primary_selector, snapshot),
                );
            }
            let cheap_selector = provider_slot_selector(slug, ProviderModelRole::Cheap);
            if provider_role_target(&snapshot.providers, slug, ProviderModelRole::Cheap).is_some() {
                push(
                    cheap_selector.clone(),
                    self.route_target_cost_per_m_usd(&cheap_selector, snapshot),
                );
            }
        }

        candidates
    }

    fn route_target_cost_per_m_usd(
        &self,
        target: &str,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Option<f64> {
        let provider = self
            .direct_provider_for_route_target(target, snapshot)
            .ok()?;
        let (input_cost, output_cost) = provider.cost_per_token();
        ((input_cost + output_cost) * rust_decimal::Decimal::from(1_000_000u64)).to_f64()
    }
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

fn simulate_cheap_split_route(
    snapshot: &LlmRuntimeSnapshot,
    ctx: &RoutingContext,
    prompt: Option<&str>,
) -> (String, String) {
    if ctx.has_tools {
        return (
            "primary".to_string(),
            "Cheap split mode always routes tool-use requests to the primary model".to_string(),
        );
    }

    let simulated_prompt = prompt
        .map(str::to_string)
        .unwrap_or_else(|| "x".repeat((ctx.estimated_input_tokens as usize).saturating_mul(4)));
    let complexity = classify_message(&simulated_prompt, &SmartRoutingConfig::default());
    let cheap_target = snapshot
        .providers
        .cheap_model
        .clone()
        .unwrap_or_else(|| "primary".to_string());

    match complexity {
        TaskComplexity::Simple => (
            cheap_target,
            "Cheap split mode classified this request as simple".to_string(),
        ),
        TaskComplexity::Moderate => {
            let target = if snapshot.providers.smart_routing_cascade {
                cheap_target
            } else {
                cheap_target
            };
            let reason = if snapshot.providers.smart_routing_cascade {
                "Cheap split mode classified this request as moderate, so it starts on the cheap model and may cascade to primary if the answer looks uncertain".to_string()
            } else {
                "Cheap split mode classified this request as moderate, so it stays on the cheap model".to_string()
            };
            (target, reason)
        }
        TaskComplexity::Complex => (
            "primary".to_string(),
            "Cheap split mode classified this request as complex".to_string(),
        ),
    }
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

pub fn normalize_providers_settings(settings: &Settings) -> ProvidersSettings {
    normalize_providers_settings_from_parts(
        settings.providers.clone(),
        legacy_primary_slug(settings),
        settings.selected_model.clone(),
    )
}

fn normalize_providers_settings_from_parts(
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
        let slots = providers
            .provider_models
            .entry(primary_slug)
            .or_insert_with(ProviderModelSlots::default);
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
            .or_insert_with(ProviderModelSlots::default);
        if slots.cheap.is_none() {
            slots.cheap = Some(model.to_string());
        }
        if providers.preferred_cheap_provider.is_none() {
            providers.preferred_cheap_provider = Some(slug.to_string());
        }
    }
    for (slug, allowed) in &providers.allowed_models {
        if let Some(model) = allowed.first() {
            let slots = providers
                .provider_models
                .entry(slug.clone())
                .or_insert_with(ProviderModelSlots::default);
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
        let slots = providers
            .provider_models
            .entry(slug.clone())
            .or_insert_with(ProviderModelSlots::default);

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
    if let Some(primary_slug) = primary {
        if let Some(candidate_model) = suggest_provider_cheap_model(primary_slug, primary_model)
            && primary_model != Some(candidate_model.as_str())
        {
            let candidate = format!("{primary_slug}/{candidate_model}");
            return Some(candidate);
        }
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
        "anthropic" => Some("claude-3-5-haiku-latest"),
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
        .map(|endpoint| endpoint.default_model)
        .or_else(|| match slug {
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

    #[test]
    fn resolved_completion_request_clears_model_override() {
        let request = CompletionRequest::new(vec![crate::llm::ChatMessage::user("hi")])
            .with_model("openai/gpt-5.4-mini");

        let resolved = RuntimeLlmProvider::resolved_completion_request(request);

        assert!(resolved.model.is_none());
    }

    #[test]
    fn resolved_tool_completion_request_clears_model_override() {
        let request =
            ToolCompletionRequest::new(vec![crate::llm::ChatMessage::user("hi")], Vec::new())
                .with_model("openai/gpt-5.4-mini");

        let resolved = RuntimeLlmProvider::resolved_tool_completion_request(request);

        assert!(resolved.model.is_none());
    }

    #[test]
    fn normalize_promotes_legacy_models_into_provider_slots() {
        let mut settings = Settings::default();
        settings.llm_backend = Some("openai".to_string());
        settings.selected_model = Some("gpt-4o".to_string());
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
                primary: Some("claude-sonnet-4-20250514".to_string()),
                cheap: Some("claude-3-5-haiku-latest".to_string()),
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
                primary: Some("claude-sonnet-4-20250514".to_string()),
                cheap: Some("claude-3-5-haiku-latest".to_string()),
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
                primary: Some("claude-sonnet-4-20250514".to_string()),
                cheap: Some("claude-3-5-haiku-latest".to_string()),
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
}
