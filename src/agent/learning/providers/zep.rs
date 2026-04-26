use super::*;
#[async_trait]
impl MemoryProvider for ZepProvider {
    fn name(&self) -> &'static str {
        "zep"
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

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client
            .post(format!("{}/api/v1/search", base_url.trim_end_matches('/')))
            .json(&serde_json::json!({
                "user_id": user_id,
                "query": query,
                "limit": limit,
            }));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }

        let response = req.send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Zep search failed: HTTP {}", response.status()));
        }
        let json = response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| e.to_string())?;
        let hits = json
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
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

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client
            .post(format!("{}/api/v1/events", base_url.trim_end_matches('/')))
            .json(&serde_json::json!({
                "user_id": user_id,
                "payload": payload,
            }));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        let response = req.send().await.map_err(|e| e.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("Zep export failed: HTTP {}", response.status()))
        }
    }
}
