use super::*;
#[async_trait]
impl MemoryProvider for Mem0Provider {
    fn name(&self) -> &'static str {
        "mem0"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        let provider = settings.providers.provider(self.name());
        let default_base = Some("https://api.mem0.ai");
        let required = if provider
            .and_then(|provider| provider_base_url(&provider.config))
            .is_none_or(|url| url.contains("api.mem0.ai"))
        {
            vec!["api_key"]
        } else {
            Vec::new()
        };
        configured_provider_health(
            self.name(),
            provider,
            default_base,
            None,
            "token",
            &required,
        )
        .await
    }

    async fn system_prompt_block(
        &self,
        settings: &LearningSettings,
        user_id: &str,
    ) -> Option<String> {
        let provider = settings.providers.provider(self.name())?;
        if !provider.enabled || !provider.user_modeling_enabled {
            return None;
        }
        Some(format!(
            "## Mem0 Memory\nActive for user {}. Use external_memory_recall for semantic memory lookup.",
            provider_scoped_user_id(&provider.config, user_id)
        ))
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
            .ok_or_else(|| "mem0 provider is not configured".to_string())?;
        if !provider.enabled {
            return Ok(Vec::new());
        }
        let base_url = provider_base_url_or(&provider.config, "https://api.mem0.ai");
        let path = provider_path(&provider.config, "search_path", "/v2/memories/search/");
        let url = provider_join_url(&base_url, &path);
        let scoped_user_id = provider_scoped_user_id(&provider.config, user_id);
        let mut body = serde_json::json!({
            "query": query,
            "filters": {"user_id": scoped_user_id},
            "user_id": scoped_user_id,
            "top_k": limit,
            "limit": limit,
        });
        if provider_bool(&provider.config, "rerank")
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert("rerank".to_string(), serde_json::json!(true));
        }
        let response = provider_json_request(
            &provider.config,
            "token",
            reqwest::Method::POST,
            &url,
            Some(body),
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
        let base_url = provider_base_url_or(&provider.config, "https://api.mem0.ai");
        let path = provider_path(&provider.config, "sync_path", "/v1/memories/");
        let url = provider_join_url(&base_url, &path);
        let scoped_user_id = provider_scoped_user_id(&provider.config, user_id);
        let body = serde_json::json!({
            "messages": provider_export_messages(&provider.config, payload),
            "user_id": scoped_user_id,
            "agent_id": provider_agent_id(&provider.config),
            "metadata": {"source": "thinclaw"},
        });
        let _ = provider_json_request(
            &provider.config,
            "token",
            reqwest::Method::POST,
            &url,
            Some(body),
        )
        .await?;
        Ok(())
    }
}
