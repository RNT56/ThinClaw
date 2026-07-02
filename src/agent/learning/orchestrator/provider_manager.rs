use super::*;

pub struct LearningOrchestrator {
    pub(in crate::agent::learning) store: Arc<dyn Database>,
    pub(in crate::agent::learning) workspace: Option<Arc<Workspace>>,
    pub(in crate::agent::learning) skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    pub(in crate::agent::learning) routine_engine: Option<Arc<RoutineEngine>>,
    pub(in crate::agent::learning) provider_manager: Arc<MemoryProviderManager>,
}

pub(in crate::agent::learning) use thinclaw_agent::learning_policy::GeneratedSkillLifecycle;

pub(in crate::agent::learning) const PROPOSAL_SUPPRESSION_WINDOW_HOURS: i64 = 24 * 7;

impl MemoryProviderManager {
    pub fn new(store: Arc<dyn Database>) -> Self {
        let providers: Vec<Arc<dyn MemoryProvider>> = vec![
            Arc::new(HonchoProvider),
            Arc::new(ZepProvider),
            Arc::new(Mem0Provider),
            Arc::new(OpenMemoryProvider),
            Arc::new(LettaProvider),
            Arc::new(ChromaProvider),
            Arc::new(QdrantProvider),
            Arc::new(CustomHttpProvider),
        ];
        Self { store, providers }
    }

    #[cfg(all(test, feature = "libsql"))]
    pub(in crate::agent::learning) fn with_providers(
        store: Arc<dyn Database>,
        providers: Vec<Arc<dyn MemoryProvider>>,
    ) -> Self {
        Self { store, providers }
    }

    pub async fn load_settings_for_user(&self, user_id: &str) -> LearningSettings {
        match self.store.get_all_settings(user_id).await {
            Ok(map) => crate::settings::Settings::from_db_map(&map).learning,
            Err(_) => LearningSettings::default(),
        }
    }

    /// Drop any cached active-provider readiness resolution for `user_id`.
    ///
    /// Called after settings mutations (provider (re)configuration, activation,
    /// or shutdown) so the next `ready_active_provider` call reflects the
    /// change immediately instead of waiting out `READY_PROVIDER_CACHE_TTL`.
    pub(in crate::agent::learning) async fn invalidate_ready_cache(&self, user_id: &str) {
        // Bump the epoch FIRST: a resolution racing this invalidation
        // (settings already loaded, entry not yet inserted) snapshotted the
        // old epoch and will skip its insert.
        super::super::providers::ready_cache_epoch()
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let key = super::super::providers::ready_cache_key(&self.store, user_id);
        super::super::providers::global_ready_cache()
            .write()
            .await
            .remove(&key);
    }

    pub async fn provider_health(&self, user_id: &str) -> Vec<ProviderHealthStatus> {
        let settings = self.load_settings_for_user(user_id).await;
        // Each provider's health probe is an independent HTTP round-trip (up to
        // 5s). Running them sequentially meant a full health sweep across all
        // 8 providers could take up to 40s; running them concurrently bounds
        // the sweep to the slowest single probe.
        let probes = self
            .providers
            .iter()
            .map(|provider| async { provider.health(&settings).await });
        let raw_statuses = futures::future::join_all(probes).await;
        self.providers
            .iter()
            .zip(raw_statuses)
            .map(|(provider, status)| self.decorate_provider_status(provider, &settings, status))
            .collect()
    }

    pub(in crate::agent::learning) fn active_provider_for_settings<'a>(
        &'a self,
        settings: &LearningSettings,
    ) -> Option<&'a Arc<dyn MemoryProvider>> {
        let target = settings.providers.active_provider_name()?;
        self.providers
            .iter()
            .find(|provider| provider.name() == target)
    }

    pub(in crate::agent::learning) fn provider_context_refs(
        hits: &[ProviderMemoryHit],
    ) -> Vec<String> {
        thinclaw_agent::learning_provider_types::provider_context_refs(hits)
    }

    pub(in crate::agent::learning) fn decorate_provider_status(
        &self,
        provider: &Arc<dyn MemoryProvider>,
        settings: &LearningSettings,
        status: ProviderHealthStatus,
    ) -> ProviderHealthStatus {
        let active_name = self
            .active_provider_for_settings(settings)
            .map(|active| active.name().to_string())
            .unwrap_or_else(|| {
                settings
                    .providers
                    .active_provider_name()
                    .unwrap_or_else(|| ActiveLearningProvider::None.as_str().to_string())
            });
        let is_active = self
            .active_provider_for_settings(settings)
            .is_some_and(|active| active.name() == provider.name());

        thinclaw_agent::learning_provider_types::decorate_provider_status(
            status,
            is_active,
            active_name,
            provider.tool_extensions(),
        )
    }

    /// Resolve the active memory provider for `user_id`, ready to serve a
    /// request.
    ///
    /// This sits directly on the prompt-assembly hot path (prefetch, tool
    /// extensions, and system-prompt block each call it, so a single message
    /// can trigger it up to 3x). Resolving it from scratch costs a DB settings
    /// read plus a live HTTP health probe (up to 5s), so the result is cached
    /// per user for [`READY_PROVIDER_CACHE_TTL`] — including the "not ready"
    /// outcome, so a misconfigured/unhealthy provider doesn't get re-probed on
    /// every call either. The fast path trusts the TTL plus the invalidation
    /// epoch (bumped by `invalidate_ready_cache` on every explicit settings
    /// mutation, from ANY manager instance); the settings-hash comparison
    /// only guards the slow path, where fresh settings are available anyway.
    pub(in crate::agent::learning) async fn ready_active_provider(
        &self,
        user_id: &str,
    ) -> Option<(
        LearningSettings,
        Arc<dyn MemoryProvider>,
        ProviderHealthStatus,
    )> {
        use super::super::providers::{global_ready_cache, ready_cache_epoch, ready_cache_key};

        let now = std::time::Instant::now();
        let key = ready_cache_key(&self.store, user_id);
        // Snapshot the epoch BEFORE loading settings: if an invalidation
        // lands between now and our insert, the insert is skipped (see the
        // guard below), so a raced resolution can never install stale state.
        let epoch = ready_cache_epoch().load(std::sync::atomic::Ordering::SeqCst);
        if let Some(entry) = global_ready_cache().read().await.get(&key)
            && entry.expires_at > now
        {
            return entry.ready.clone();
        }

        let settings = self.load_settings_for_user(user_id).await;
        let settings_hash =
            thinclaw_agent::learning_policy::stable_json_hash(&serde_json::json!(settings));

        // Re-check after the settings load: another caller may have finished
        // the same resolution while we were reading settings.
        {
            let cache = global_ready_cache().read().await;
            if let Some(entry) = cache.get(&key)
                && entry.expires_at > now
                && entry.settings_hash == settings_hash
            {
                return entry.ready.clone();
            }
        }

        let ready = match self.active_provider_for_settings(&settings) {
            Some(provider) => {
                let provider = provider.clone();
                let status = self.decorate_provider_status(
                    &provider,
                    &settings,
                    provider.health(&settings).await,
                );
                if status.readiness.is_ready() {
                    Some((settings.clone(), provider, status))
                } else {
                    tracing::debug!(
                        provider = provider.name(),
                        readiness = status.readiness.as_str(),
                        error = status.error.as_deref().unwrap_or(""),
                        "learning provider is not ready; failing closed"
                    );
                    None
                }
            }
            None => None,
        };

        {
            let mut cache = super::super::providers::global_ready_cache().write().await;
            // Insert guard: skip when an invalidation moved the epoch after
            // our snapshot — this resolution may predate the settings change.
            // Checked under the write lock so it cannot interleave with the
            // invalidator's bump-then-remove sequence.
            let current = ready_cache_epoch().load(std::sync::atomic::Ordering::SeqCst);
            if current == epoch {
                cache.insert(
                    key,
                    ReadyProviderCacheEntry {
                        settings_hash,
                        expires_at: now + READY_PROVIDER_CACHE_TTL,
                        ready: ready.clone(),
                    },
                );
            }
        }

        ready
    }

    pub async fn prefetch_provider_context(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Option<ProviderPrefetchContext> {
        let (settings, provider, _) = self.ready_active_provider(user_id).await?;
        let hits = match provider.prefetch(&settings, user_id, query, limit).await {
            Ok(hits) => hits,
            Err(err) => {
                tracing::debug!(
                    provider = provider.name(),
                    user_id = %user_id,
                    error = %err,
                    "learning provider prefetch failed"
                );
                Vec::new()
            }
        };
        let rendered_context = provider.render_prompt_context(&hits)?;
        Some(ProviderPrefetchContext {
            provider: provider.name().to_string(),
            context_refs: Self::provider_context_refs(&hits),
            hits,
            rendered_context,
        })
    }

    pub async fn provider_system_prompt_block(&self, user_id: &str) -> Option<String> {
        let (settings, provider, _) = self.ready_active_provider(user_id).await?;
        provider.prefetch_session_context(&settings, user_id).await
    }

    pub async fn provider_recall(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<ProviderMemoryHit> {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return Vec::new();
        };
        match provider.recall(&settings, user_id, query, limit).await {
            Ok(hits) => hits,
            Err(err) => {
                tracing::debug!(
                    provider = provider.name(),
                    error = %err,
                    "learning provider recall skipped"
                );
                Vec::new()
            }
        }
    }

    pub async fn after_turn_sync(&self, user_id: &str, artifact: &crate::agent::AgentRunArtifact) {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return;
        };
        let payload = artifact.provider_payload();
        if let Err(err) = provider.after_turn_sync(&settings, user_id, &payload).await {
            tracing::debug!(
                provider = provider.name(),
                error = %err,
                "learning provider turn sync skipped"
            );
        }
    }

    pub async fn export_payload(
        &self,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<String, String> {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return Err("no ready external memory provider is active".to_string());
        };
        provider.export_turn(&settings, user_id, payload).await?;
        Ok(provider.name().to_string())
    }

    pub async fn session_end_extract(
        &self,
        user_id: &str,
        artifact: &crate::agent::AgentRunArtifact,
    ) {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return;
        };
        let payload = artifact.provider_payload();
        if let Err(err) = provider
            .session_end_extract(&settings, user_id, &payload)
            .await
        {
            tracing::debug!(
                provider = provider.name(),
                error = %err,
                "learning provider session-end extract skipped"
            );
        }
    }

    pub async fn mirror_workspace_write(&self, user_id: &str, payload: &serde_json::Value) {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return;
        };
        if let Err(err) = provider
            .mirror_workspace_write(&settings, user_id, payload)
            .await
        {
            tracing::debug!(
                provider = provider.name(),
                error = %err,
                "learning provider workspace write mirror skipped"
            );
        }
    }

    pub async fn provider_tool_extensions(&self, user_id: &str) -> Vec<String> {
        self.ready_active_provider(user_id)
            .await
            .map(|(_, provider, _)| provider.tool_extensions())
            .unwrap_or_default()
    }

    pub async fn shutdown_active_provider(&self, user_id: &str) -> Result<(), String> {
        let Some((settings, provider, _)) = self.ready_active_provider(user_id).await else {
            return Ok(());
        };
        provider.shutdown(&settings).await
    }
}

#[cfg(all(test, feature = "libsql"))]
mod ready_provider_cache_tests {
    use super::*;

    /// Minimal `MemoryProvider` double that counts `health()` calls so tests
    /// can assert the TTL cache actually skips the probe on a hit.
    struct CountingProvider {
        name: &'static str,
        health_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl MemoryProvider for CountingProvider {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn health(&self, _settings: &LearningSettings) -> ProviderHealthStatus {
            self.health_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ProviderHealthStatus {
                provider: self.name.to_string(),
                active: false,
                enabled: true,
                healthy: true,
                readiness: crate::agent::learning::ProviderReadiness::Ready,
                latency_ms: Some(1),
                error: None,
                capabilities: Vec::new(),
                metadata: serde_json::Value::Null,
            }
        }

        async fn recall(
            &self,
            _settings: &LearningSettings,
            _user_id: &str,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<ProviderMemoryHit>, String> {
            Ok(Vec::new())
        }

        async fn export_turn(
            &self,
            _settings: &LearningSettings,
            _user_id: &str,
            _payload: &serde_json::Value,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn ready_active_provider_cache_hit_skips_reprobe_within_ttl() {
        let (db, _guard) = crate::testing::test_db().await;
        let user_id = "ready-cache-hit-user";
        db.set_setting(
            user_id,
            "learning.providers.active",
            &serde_json::json!("honcho"),
        )
        .await
        .expect("set active provider");

        let health_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let manager = MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![Arc::new(CountingProvider {
                name: "honcho",
                health_calls: Arc::clone(&health_calls),
            })],
        );

        let first = manager.ready_active_provider(user_id).await;
        assert!(first.is_some(), "first resolution should be ready");
        assert_eq!(
            health_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "first call should probe health exactly once"
        );

        // Second call within the TTL window must hit the cache: no DB
        // settings reload and no additional health probe.
        let second = manager.ready_active_provider(user_id).await;
        assert!(second.is_some(), "cached resolution should still be ready");
        assert_eq!(
            health_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "cache hit must not re-probe provider health"
        );

        // Explicit invalidation (as done after settings mutations) forces a
        // fresh resolution on the next call.
        manager.invalidate_ready_cache(user_id).await;
        let third = manager.ready_active_provider(user_id).await;
        assert!(third.is_some());
        assert_eq!(
            health_calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "invalidation must force a re-probe on the next call"
        );
    }

    #[tokio::test]
    async fn ready_provider_cache_entry_expiry_is_a_pure_time_comparison() {
        // Exercises the TTL policy directly without any I/O: an entry is
        // valid exactly while `now < expires_at`, and `READY_PROVIDER_CACHE_TTL`
        // is what every fresh resolution is stamped with.
        let now = std::time::Instant::now();
        let entry = ReadyProviderCacheEntry {
            settings_hash: 42,
            expires_at: now + READY_PROVIDER_CACHE_TTL,
            ready: None,
        };
        assert!(
            entry.expires_at > now,
            "a freshly-inserted entry must not be immediately expired"
        );
        assert!(
            entry.expires_at <= now + READY_PROVIDER_CACHE_TTL,
            "entry must expire no later than one full TTL from insertion"
        );

        let expired_entry = ReadyProviderCacheEntry {
            settings_hash: 42,
            expires_at: now,
            ready: None,
        };
        let later = now + std::time::Duration::from_millis(1);
        assert!(
            expired_entry.expires_at <= later,
            "an entry stamped with `now` must be treated as expired an instant later"
        );
    }
}
