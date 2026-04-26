use super::*;
#[async_trait]
impl MemoryProvider for LettaProvider {
    fn name(&self) -> &'static str {
        "letta"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        let provider = settings.providers.provider(self.name());
        let mut required = vec!["agent_id"];
        if provider
            .and_then(|provider| provider_base_url(&provider.config))
            .is_none_or(|url| url.contains("api.letta.com"))
        {
            required.push("api_key");
        }
        configured_provider_health(
            self.name(),
            provider,
            Some("https://api.letta.com"),
            None,
            "bearer",
            &required,
        )
        .await
    }

    async fn recall(
        &self,
        settings: &LearningSettings,
        _user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String> {
        let provider = settings
            .providers
            .provider(self.name())
            .ok_or_else(|| "letta provider is not configured".to_string())?;
        if !provider.enabled {
            return Ok(Vec::new());
        }
        let base_url = provider_base_url_or(&provider.config, "https://api.letta.com");
        let path = provider_path_with_vars(
            &provider.config,
            "search_path",
            "/v1/agents/{agent_id}/archival-memory/search",
        );
        let url = provider_join_url(&base_url, &path);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|error| error.to_string())?;
        let request = apply_provider_auth(
            client
                .get(&url)
                .query(&[("query", query.to_string()), ("topK", limit.to_string())]),
            &provider.config,
            "bearer",
        );
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status();
        let text = response.text().await.map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {text}"));
        }
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(parse_provider_hits(value, self.name()))
    }

    async fn export_turn(
        &self,
        settings: &LearningSettings,
        _user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        let Some(provider) = settings.providers.provider(self.name()) else {
            return Ok(());
        };
        if !provider.enabled {
            return Ok(());
        }
        let base_url = provider_base_url_or(&provider.config, "https://api.letta.com");
        let path = provider_path_with_vars(
            &provider.config,
            "sync_path",
            "/v1/agents/{agent_id}/archival-memory",
        );
        let url = provider_join_url(&base_url, &path);
        let tags = provider_config_value(&provider.config, "tags")
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec!["thinclaw".to_string(), "memory_export".to_string()]);
        let _ = provider_json_request(
            &provider.config,
            "bearer",
            reqwest::Method::POST,
            &url,
            Some(serde_json::json!({
                "content": payload_text(payload),
                "text": payload_text(payload),
                "tags": tags,
            })),
        )
        .await?;
        Ok(())
    }
}
