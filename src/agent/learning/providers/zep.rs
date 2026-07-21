use super::*;
#[async_trait]
impl MemoryProvider for ZepProvider {
    fn name(&self) -> &'static str {
        "zep"
    }

    fn supports_strict_subject_scoping(&self) -> bool {
        true
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        provider_health_request(
            self.name(),
            settings.providers.zep.enabled,
            provider_base_url(&settings.providers.zep.config),
            provider_token(&settings.providers.zep.config),
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
        if !settings.providers.zep.enabled {
            return Ok(Vec::new());
        }
        let base_url = provider_base_url(&settings.providers.zep.config)
            .ok_or_else(|| "Zep base_url not configured".to_string())?;
        let token = provider_token(&settings.providers.zep.config);

        let url = provider_join_url(&base_url, "/api/v1/search")?;
        let mut req = shared_http_client()?
            .post(url)
            .timeout(std::time::Duration::from_secs(8))
            .json(&serde_json::json!({
                "user_id": user_id,
                "query": query,
                "limit": limit,
            }));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }

        let response = req.send().await.map_err(|e| e.without_url().to_string())?;
        let json = provider_json_response(response).await?;
        let hits = json
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(limit.min(100))
            .map(|item| ProviderMemoryHit {
                provider: self.name().to_string(),
                summary: item
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("content").and_then(|v| v.as_str()))
                    .unwrap_or_default()
                    .to_string(),
                score: item.get("score").and_then(|v| v.as_f64()),
                provenance: item,
            })
            .collect();
        Ok(hits)
    }

    async fn export_turn(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        if !settings.providers.zep.enabled {
            return Ok(());
        }
        let base_url = provider_base_url(&settings.providers.zep.config)
            .ok_or_else(|| "Zep base_url not configured".to_string())?;
        let token = provider_token(&settings.providers.zep.config);

        let url = provider_join_url(&base_url, "/api/v1/events")?;
        let mut req = shared_http_client()?
            .post(url)
            .timeout(std::time::Duration::from_secs(8))
            .json(&serde_json::json!({
                "user_id": user_id,
                "payload": payload,
            }));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        let response = req.send().await.map_err(|e| e.without_url().to_string())?;
        provider_status_response(response).await
    }
}
