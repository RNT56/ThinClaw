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

    #[cfg(test)]
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

    pub async fn provider_health(&self, user_id: &str) -> Vec<ProviderHealthStatus> {
        let settings = self.load_settings_for_user(user_id).await;
        let mut statuses = Vec::new();
        for provider in &self.providers {
            let status = self.decorate_provider_status(
                provider,
                &settings,
                provider.health(&settings).await,
            );
            statuses.push(status);
        }
        statuses
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

    pub(in crate::agent::learning) async fn ready_active_provider(
        &self,
        user_id: &str,
    ) -> Option<(
        LearningSettings,
        Arc<dyn MemoryProvider>,
        ProviderHealthStatus,
    )> {
        let settings = self.load_settings_for_user(user_id).await;
        let provider = self.active_provider_for_settings(&settings)?.clone();
        let status =
            self.decorate_provider_status(&provider, &settings, provider.health(&settings).await);
        if !status.readiness.is_ready() {
            tracing::debug!(
                provider = provider.name(),
                readiness = status.readiness.as_str(),
                error = status.error.as_deref().unwrap_or(""),
                "learning provider is not ready; failing closed"
            );
            return None;
        }
        Some((settings, provider, status))
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
