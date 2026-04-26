use super::*;
#[async_trait]
impl MemoryProvider for OpenMemoryProvider {
    fn name(&self) -> &'static str {
        "openmemory"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        configured_provider_health(
            self.name(),
            settings.providers.provider(self.name()),
            Some("http://localhost:8888"),
            Some("/"),
            "x-api-key",
            &[],
        )
        .await
    }

    async fn recall(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String> {
        let provider = settings
            .providers
            .provider(self.name())
            .ok_or_else(|| "openmemory provider is not configured".to_string())?;
        if !provider.enabled {
            return Ok(Vec::new());
        }
        let base_url = provider_base_url_or(&provider.config, "http://localhost:8888");
        let path = provider_path(&provider.config, "search_path", "/search");
        let url = provider_join_url(&base_url, &path);
        let scoped_user_id = provider_scoped_user_id(&provider.config, user_id);
        let response = provider_json_request(
            &provider.config,
            "x-api-key",
            reqwest::Method::POST,
            &url,
            Some(serde_json::json!({
                "query": query,
                "user_id": scoped_user_id,
                "limit": limit,
                "top_k": limit,
            })),
        )
        .await?;
        Ok(parse_provider_hits(response, self.name()))
    }

    async fn export_turn(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        let Some(provider) = settings.providers.provider(self.name()) else {
            return Ok(());
        };
        if !provider.enabled {
            return Ok(());
        }
        let base_url = provider_base_url_or(&provider.config, "http://localhost:8888");
        let path = provider_path(&provider.config, "sync_path", "/memories");
        let url = provider_join_url(&base_url, &path);
        let scoped_user_id = provider_scoped_user_id(&provider.config, user_id);
        let _ = provider_json_request(
            &provider.config,
            "x-api-key",
            reqwest::Method::POST,
            &url,
            Some(serde_json::json!({
                "messages": provider_export_messages(&provider.config, payload),
                "user_id": scoped_user_id,
                "agent_id": provider_agent_id(&provider.config),
            })),
        )
        .await?;
        Ok(())
    }
}
