use super::*;
impl LearningOrchestrator {
    pub fn new(
        store: Arc<dyn Database>,
        workspace: Option<Arc<Workspace>>,
        skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    ) -> Self {
        let provider_manager = Arc::new(MemoryProviderManager::new(Arc::clone(&store)));
        Self {
            store,
            workspace,
            skill_registry,
            routine_engine: None,
            provider_manager,
        }
    }

    pub fn with_routine_engine(mut self, routine_engine: Option<Arc<RoutineEngine>>) -> Self {
        self.routine_engine = routine_engine;
        self
    }

    pub fn memory_provider_manager(&self) -> Arc<MemoryProviderManager> {
        Arc::clone(&self.provider_manager)
    }

    pub async fn load_settings_for_user(&self, user_id: &str) -> LearningSettings {
        match self.store.get_all_settings(user_id).await {
            Ok(map) => {
                let settings = crate::settings::Settings::from_db_map(&map);
                let mut learning = settings.learning;
                if settings.desktop_autonomy.is_reckless_enabled() {
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "memory");
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "skill");
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "prompt");
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "routine");
                    ensure_auto_apply_class(&mut learning.auto_apply_classes, "code");
                    learning.skill_synthesis.auto_apply = true;
                    learning.code_proposals.auto_apply_without_review = true;
                    learning.code_proposals.publish_mode = "local_autorollout".to_string();
                }
                learning
            }
            Err(_) => LearningSettings::default(),
        }
    }

    pub(in crate::agent::learning) async fn load_full_settings_for_user(
        &self,
        user_id: &str,
    ) -> crate::settings::Settings {
        match self.store.get_all_settings(user_id).await {
            Ok(map) => crate::settings::Settings::from_db_map(&map),
            Err(_) => crate::settings::Settings::default(),
        }
    }

    pub(in crate::agent::learning) async fn persist_full_settings(
        &self,
        user_id: &str,
        settings: &crate::settings::Settings,
    ) -> Result<(), String> {
        for (key, value) in settings.to_db_map() {
            self.store
                .set_setting(user_id, &key, &value)
                .await
                .map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    pub async fn configure_memory_provider(
        &self,
        user_id: &str,
        provider_name: &str,
        provider_settings: crate::settings::LearningProviderSettings,
        activate: bool,
    ) -> Result<Vec<ProviderHealthStatus>, String> {
        let provider_name = provider_name.trim().to_ascii_lowercase();
        if provider_name.is_empty() {
            return Err("provider name must be non-empty".to_string());
        }

        let mut settings = self.load_full_settings_for_user(user_id).await;
        *settings.learning.providers.provider_mut(&provider_name) = provider_settings;
        if activate {
            settings.learning.providers.active_provider = Some(provider_name.clone());
            settings.learning.providers.active = match provider_name.as_str() {
                "honcho" => ActiveLearningProvider::Honcho,
                "zep" => ActiveLearningProvider::Zep,
                _ => ActiveLearningProvider::None,
            };
        }
        self.persist_full_settings(user_id, &settings).await?;
        Ok(self.provider_health(user_id).await)
    }

    pub async fn disable_active_memory_provider(
        &self,
        user_id: &str,
    ) -> Result<Vec<ProviderHealthStatus>, String> {
        self.provider_manager
            .shutdown_active_provider(user_id)
            .await?;
        let mut settings = self.load_full_settings_for_user(user_id).await;
        settings.learning.providers.active = ActiveLearningProvider::None;
        settings.learning.providers.active_provider = None;
        self.persist_full_settings(user_id, &settings).await?;
        Ok(self.provider_health(user_id).await)
    }

    pub async fn provider_health(&self, user_id: &str) -> Vec<ProviderHealthStatus> {
        self.provider_manager.provider_health(user_id).await
    }

    pub async fn prefetch_provider_context(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Option<ProviderPrefetchContext> {
        self.provider_manager
            .prefetch_provider_context(user_id, query, limit)
            .await
    }

    pub async fn provider_recall(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<ProviderMemoryHit> {
        self.provider_manager
            .provider_recall(user_id, query, limit)
            .await
    }

    pub async fn provider_system_prompt_block(&self, user_id: &str) -> Option<String> {
        self.provider_manager
            .provider_system_prompt_block(user_id)
            .await
    }

    pub async fn after_turn_sync_to_provider(
        &self,
        user_id: &str,
        artifact: &crate::agent::AgentRunArtifact,
    ) {
        self.provider_manager
            .after_turn_sync(user_id, artifact)
            .await;
    }

    pub async fn export_provider_payload(
        &self,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<String, String> {
        self.provider_manager.export_payload(user_id, payload).await
    }

    pub async fn session_end_extract(
        &self,
        user_id: &str,
        artifact: &crate::agent::AgentRunArtifact,
    ) {
        self.provider_manager
            .session_end_extract(user_id, artifact)
            .await;
    }

    pub async fn mirror_workspace_write(&self, user_id: &str, payload: &serde_json::Value) {
        self.provider_manager
            .mirror_workspace_write(user_id, payload)
            .await;
    }

    pub async fn provider_tool_extensions(&self, user_id: &str) -> Vec<String> {
        self.provider_manager
            .provider_tool_extensions(user_id)
            .await
    }
}
