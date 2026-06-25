//! `LlmRuntimeManager`: the central runtime state holder. Owns the live config
//! snapshot, routing policy/planner, per-target provider caches, and route
//! health. This module covers construction, status reporting, hot reload, and
//! the public provider handles. Routing/resolution and simulation methods live
//! in sibling submodules as additional `impl` blocks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::Config;
use crate::db::Database;
use crate::error::LlmError;
use crate::llm::provider::LlmProvider;
use crate::llm::provider_factory::build_provider_chain;
use crate::llm::route_planner::{
    RoutePlanner, validate_providers_settings as validate_planner_settings,
};
use crate::llm::routing_policy::{RouteCandidate, RoutingPolicy};
use crate::secrets::SecretsStore;
use crate::settings::{ProvidersSettings, RoutingMode, Settings};

use super::credentials::hydrate_runtime_credentials_from_secrets;
use super::provider_build::{build_routing_policy, legacy_primary_slug_from_config};
use super::settings_defaults::{derive_runtime_defaults_from_parts, normalize_providers_settings};
use super::types::{
    AdvisorReadyCallback, LlmRuntimeSnapshot, RuntimeLlmProvider, RuntimeProviderRole,
    RuntimeStatus,
};

pub struct LlmRuntimeManager {
    pub(super) user_id: String,
    pub(super) db: Option<Arc<dyn Database>>,
    pub(super) secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    pub(super) toml_path: Option<PathBuf>,
    pub(super) snapshot: RwLock<LlmRuntimeSnapshot>,
    pub routing_policy: Arc<RwLock<RoutingPolicy>>,
    pub route_planner: Arc<RwLock<RoutePlanner>>,
    /// Prebuilt per-target providers (reused across requests).
    pub(super) target_provider_cache: RwLock<HashMap<String, Arc<dyn LlmProvider>>>,
    /// Prebuilt failover/retry chains keyed by ordered target sequence.
    pub(super) chain_provider_cache: RwLock<HashMap<String, Arc<dyn LlmProvider>>>,
    /// Static candidate metadata + costs, rebuilt on startup/reload.
    pub(super) route_candidate_cache: RwLock<Vec<RouteCandidate>>,
    /// Live route-health EMA keyed by canonical telemetry key.
    pub(super) route_health: RwLock<HashMap<String, f64>>,
    /// Last observed dynamic pricing revision used to hydrate candidate costs.
    pub(super) dynamic_pricing_revision_seen: AtomicU64,
    pub(super) revision: AtomicU64,
    pub(super) last_error: RwLock<Option<String>>,
    pub(super) advisor_ready_callback: RwLock<Option<AdvisorReadyCallback>>,
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

    pub(super) fn notify_advisor_ready_callback(&self) {
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

    pub(super) fn snapshot(&self) -> LlmRuntimeSnapshot {
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
            let (mut config, mut providers) =
                self.load_runtime_inputs()
                    .await
                    .map_err(|reason| LlmError::RequestFailed {
                        provider: "runtime".to_string(),
                        reason,
                    })?;
            hydrate_runtime_credentials_from_secrets(
                &mut config,
                &mut providers,
                self.secrets_store.as_ref(),
                &self.user_id,
            )
            .await;
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
}
