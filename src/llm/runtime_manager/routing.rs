//! Route resolution and the provider-chain hot path: turning route targets
//! (`primary`/`cheap`/`slug@role`/`slug/model`) into concrete (possibly
//! failover/retry/circuit-breaker/cache-wrapped) providers, building and
//! caching route candidates, tracking route health, and pricing.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rust_decimal::prelude::ToPrimitive;

use crate::error::LlmError;
use crate::llm::provider::LlmProvider;
use crate::llm::provider_factory::create_llm_provider;
use crate::llm::routing_policy::{RouteCandidate, policy_rule_targets};
use crate::llm::{
    CachedProvider, CircuitBreakerConfig, CircuitBreakerProvider, CooldownConfig, FailoverProvider,
    ResponseCacheConfig, RetryConfig, RetryProvider,
};

use super::manager::LlmRuntimeManager;
use super::provider_build::{
    apply_model_override, capability_metadata_for_route, create_provider_for_runtime_slug,
    logical_role_for_target,
};
use super::provider_slots::{
    parse_provider_slot_selector, provider_role_target, provider_slot_selector,
    provider_slot_selectors,
};
use super::types::{LlmRuntimeSnapshot, ProviderModelRole};

impl LlmRuntimeManager {
    pub(super) fn route_health_snapshot(&self) -> HashMap<String, f64> {
        self.route_health
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(super) fn record_route_outcome(
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

    pub(super) fn refresh_route_caches(&self) -> Result<(), LlmError> {
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

    pub(super) fn resolve_route_model_spec(
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

    pub(super) fn resolve_telemetry_key_for_target(
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

    pub(super) fn provider_for_route_target(
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

    pub(super) fn provider_chain_for_targets(
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

    pub(super) fn provider_for_model_spec(
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
            let provider = create_provider_for_runtime_slug(
                provider,
                model,
                &snapshot.config.llm,
                Some(&snapshot.providers),
            )?;
            return Ok(Self::wrap_runtime_provider_with_retry(provider, snapshot));
        }

        if let Some(primary_provider) = snapshot.providers.primary.as_deref() {
            let provider = create_provider_for_runtime_slug(
                primary_provider,
                spec,
                &snapshot.config.llm,
                Some(&snapshot.providers),
            )?;
            return Ok(Self::wrap_runtime_provider_with_retry(provider, snapshot));
        }

        let mut llm_config = snapshot.config.llm.clone();
        apply_model_override(&mut llm_config, spec);
        let provider = create_llm_provider(&llm_config)?;
        Ok(Self::wrap_runtime_provider_with_retry(provider, snapshot))
    }

    pub fn provider_handle_for_target(
        &self,
        target: &str,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        let snapshot = self.snapshot();
        self.provider_for_route_target(target, &snapshot)
    }

    pub(super) fn available_route_candidates(
        &self,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Vec<RouteCandidate> {
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
