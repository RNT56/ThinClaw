use super::*;
#[async_trait]
impl MemoryProvider for HonchoProvider {
    fn name(&self) -> &'static str {
        "honcho"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        provider_health_request(
            self.name(),
            settings.providers.honcho.enabled,
            provider_base_url(&settings.providers.honcho.config),
            provider_token(&settings.providers.honcho.config),
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
        let base_url = provider_base_url(&provider.config)?;
        let token = provider_token(&provider.config);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .ok()?;

        let mut request = client
            .get(format!(
                "{}/v1/user-context",
                base_url.trim_end_matches('/')
            ))
            .query(&[
                ("user_id", user_id),
                ("cadence", &provider.cadence.unwrap_or(5).to_string()),
                ("depth", &provider.depth.unwrap_or(3).to_string()),
            ]);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.ok()?.error_for_status().ok()?;
        let payload = response.json::<serde_json::Value>().await.ok()?;

        let user_representations = payload
            .get("user_representations")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        let peer_cards = payload
            .get("peer_cards")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        let session_summary = payload
            .get("session_summary")
            .and_then(|value| value.as_str())
            .map(str::to_string);

        if user_representations.is_empty() && peer_cards.is_empty() && session_summary.is_none() {
            return None;
        }

        let mut lines = vec!["## External Memory Model".to_string()];
        if !user_representations.is_empty() {
            lines.push("User representations:".to_string());
            lines.extend(
                user_representations
                    .into_iter()
                    .map(|value| format!("- {value}")),
            );
        }
        if !peer_cards.is_empty() {
            lines.push("Peer cards:".to_string());
            lines.extend(peer_cards.into_iter().map(|value| format!("- {value}")));
        }
        if let Some(summary) = session_summary {
            lines.push(format!("Session summary: {summary}"));
        }
        Some(lines.join("\n"))
    }

    async fn recall(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String> {
        if !settings.providers.honcho.enabled {
            return Ok(Vec::new());
        }
        let base_url = provider_base_url(&settings.providers.honcho.config)
            .ok_or_else(|| "Honcho base_url not configured".to_string())?;
        let token = provider_token(&settings.providers.honcho.config);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client
            .post(format!("{}/v1/search", base_url.trim_end_matches('/')))
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
            return Err(format!("Honcho search failed: HTTP {}", response.status()));
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
                    .or_else(|| item.get("text").and_then(|v| v.as_str()))
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
        if !settings.providers.honcho.enabled {
            return Ok(());
        }
        let base_url = provider_base_url(&settings.providers.honcho.config)
            .ok_or_else(|| "Honcho base_url not configured".to_string())?;
        let token = provider_token(&settings.providers.honcho.config);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client
            .post(format!("{}/v1/ingest", base_url.trim_end_matches('/')))
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
            Err(format!("Honcho ingest failed: HTTP {}", response.status()))
        }
    }

    async fn pre_compress_hook(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        if !settings
            .providers
            .provider(self.name())
            .is_some_and(|provider| provider.enabled)
        {
            return Ok(());
        }
        let base_url = settings
            .providers
            .provider(self.name())
            .and_then(|provider| provider_base_url(&provider.config))
            .ok_or_else(|| "Honcho base_url not configured".to_string())?;
        let token = settings
            .providers
            .provider(self.name())
            .and_then(|provider| provider_token(&provider.config));

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;
        let mut req = client
            .post(format!(
                "{}/v1/session-summary",
                base_url.trim_end_matches('/')
            ))
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
            Err(format!(
                "Honcho session summary export failed: HTTP {}",
                response.status()
            ))
        }
    }
}
