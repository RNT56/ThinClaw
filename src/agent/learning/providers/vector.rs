use super::*;
#[async_trait]
impl MemoryProvider for ChromaProvider {
    fn name(&self) -> &'static str {
        "chroma"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        configured_provider_health(
            self.name(),
            settings.providers.provider(self.name()),
            Some("http://localhost:8000"),
            Some("/api/v2/heartbeat"),
            "x-chroma-token",
            &["collection_id", "embedding_url"],
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
            .ok_or_else(|| "chroma provider is not configured".to_string())?;
        if !provider.enabled {
            return Ok(Vec::new());
        }
        let embedding = embedding_from_config(&provider.config, query).await?;
        let base_url = provider_base_url_or(&provider.config, "http://localhost:8000");
        let path = provider_path_with_vars(
            &provider.config,
            "query_path",
            "/api/v2/tenants/{tenant}/databases/{database}/collections/{collection_id}/query",
        )
        .replace(
            "{tenant}",
            &provider_config_value(&provider.config, "tenant")
                .unwrap_or_else(|| "default_tenant".to_string()),
        )
        .replace(
            "{database}",
            &provider_config_value(&provider.config, "database")
                .unwrap_or_else(|| "default_database".to_string()),
        );
        let url = provider_join_url(&base_url, &path);
        let response = provider_json_request(
            &provider.config,
            "x-chroma-token",
            reqwest::Method::POST,
            &url,
            Some(serde_json::json!({
                "query_embeddings": [embedding],
                "n_results": limit,
                "include": ["documents", "metadatas", "distances"],
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
        let content = payload_text(payload);
        let embedding = embedding_from_config(&provider.config, &content).await?;
        let base_url = provider_base_url_or(&provider.config, "http://localhost:8000");
        let path = provider_path_with_vars(
            &provider.config,
            "sync_path",
            "/api/v2/tenants/{tenant}/databases/{database}/collections/{collection_id}/upsert",
        )
        .replace(
            "{tenant}",
            &provider_config_value(&provider.config, "tenant")
                .unwrap_or_else(|| "default_tenant".to_string()),
        )
        .replace(
            "{database}",
            &provider_config_value(&provider.config, "database")
                .unwrap_or_else(|| "default_database".to_string()),
        );
        let url = provider_join_url(&base_url, &path);
        let id = format!("thinclaw-{}", Uuid::new_v4());
        let _ = provider_json_request(
            &provider.config,
            "x-chroma-token",
            reqwest::Method::POST,
            &url,
            Some(serde_json::json!({
                "ids": [id],
                "embeddings": [embedding],
                "documents": [content],
                "metadatas": [{
                    "source": "thinclaw",
                    "user_id": user_id,
                }],
            })),
        )
        .await?;
        Ok(())
    }
}

#[async_trait]
impl MemoryProvider for QdrantProvider {
    fn name(&self) -> &'static str {
        "qdrant"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        configured_provider_health(
            self.name(),
            settings.providers.provider(self.name()),
            Some("http://localhost:6333"),
            Some("/"),
            "api-key",
            &["collection", "embedding_url"],
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
            .ok_or_else(|| "qdrant provider is not configured".to_string())?;
        if !provider.enabled {
            return Ok(Vec::new());
        }
        let embedding = embedding_from_config(&provider.config, query).await?;
        let collection = provider_config_value(&provider.config, "collection")
            .ok_or_else(|| "missing collection".to_string())?;
        let base_url = provider_base_url_or(&provider.config, "http://localhost:6333");
        let path = provider_path_with_vars(
            &provider.config,
            "query_path",
            &format!("/collections/{collection}/points/query"),
        );
        let url = provider_join_url(&base_url, &path);
        let response = provider_json_request(
            &provider.config,
            "api-key",
            reqwest::Method::POST,
            &url,
            Some(serde_json::json!({
                "query": embedding,
                "limit": limit,
                "with_payload": true,
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
        let content = payload_text(payload);
        let embedding = embedding_from_config(&provider.config, &content).await?;
        let collection = provider_config_value(&provider.config, "collection")
            .ok_or_else(|| "missing collection".to_string())?;
        let base_url = provider_base_url_or(&provider.config, "http://localhost:6333");
        let path = provider_path_with_vars(
            &provider.config,
            "sync_path",
            &format!("/collections/{collection}/points"),
        );
        let url = provider_join_url(&base_url, &path);
        let _ = provider_json_request(
            &provider.config,
            "api-key",
            reqwest::Method::PUT,
            &url,
            Some(serde_json::json!({
                "points": [{
                    "id": Uuid::new_v4().to_string(),
                    "vector": embedding,
                    "payload": {
                        "text": content,
                        "source": "thinclaw",
                        "user_id": user_id,
                    },
                }],
            })),
        )
        .await?;
        Ok(())
    }
}
