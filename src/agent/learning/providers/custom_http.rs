use super::*;
#[async_trait]
impl MemoryProvider for CustomHttpProvider {
    fn name(&self) -> &'static str {
        "custom_http"
    }

    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus {
        let Some(provider) = settings.providers.provider(self.name()) else {
            return provider_health_request(self.name(), false, None, None).await;
        };
        let base_url = provider_base_url(&provider.config);
        provider_health_request(
            self.name(),
            provider.enabled,
            base_url,
            provider_token(&provider.config),
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
            .ok_or_else(|| "custom_http provider is not configured".to_string())?;
        if !provider.enabled {
            return Ok(Vec::new());
        }
        let recall_url = provider
            .config
            .get("recall_url")
            .cloned()
            .or_else(|| {
                provider_base_url(&provider.config)
                    .map(|url| format!("{}/recall", url.trim_end_matches('/')))
            })
            .ok_or_else(|| "missing recall_url or base_url".to_string())?;
        let response = custom_http_request(
            &provider.config,
            reqwest::Method::POST,
            &recall_url,
            Some(serde_json::json!({
                "user_id": user_id,
                "query": query,
                "limit": limit,
            })),
        )
        .await?;
        Ok(parse_custom_http_hits(response, self.name()))
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
        let sync_url = provider
            .config
            .get("sync_url")
            .cloned()
            .or_else(|| {
                provider_base_url(&provider.config)
                    .map(|url| format!("{}/sync", url.trim_end_matches('/')))
            })
            .ok_or_else(|| "missing sync_url or base_url".to_string())?;
        let _ = custom_http_request(
            &provider.config,
            reqwest::Method::POST,
            &sync_url,
            Some(serde_json::json!({
                "user_id": user_id,
                "payload": payload,
            })),
        )
        .await?;
        Ok(())
    }
}

pub(super) async fn provider_json_request(
    config: &std::collections::HashMap<String, String>,
    default_auth_scheme: &str,
    method: reqwest::Method,
    url: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut request = apply_provider_auth(
        shared_http_client()
            .request(method, url)
            .timeout(std::time::Duration::from_secs(15)),
        config,
        default_auth_scheme,
    );
    if let Some(headers) = config.get("headers_json") {
        let parsed: serde_json::Map<String, serde_json::Value> = serde_json::from_str(headers)
            .map_err(|error| format!("invalid headers_json: {error}"))?;
        for (key, value) in parsed {
            if let Some(value) = value.as_str() {
                request = request.header(key, value);
            }
        }
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}"));
    }
    if text.trim().is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

pub(super) async fn embedding_from_config(
    config: &std::collections::HashMap<String, String>,
    text: &str,
) -> Result<Vec<f64>, String> {
    let embedding_url = provider_config_value(config, "embedding_url")
        .ok_or_else(|| "missing embedding_url for vector memory provider".to_string())?;
    let mut embedding_config = config.clone();
    if let Some(token) = provider_config_value(config, "embedding_api_key") {
        embedding_config.insert("api_key".to_string(), token);
    } else if let Some(env_name) = provider_config_value(config, "embedding_api_key_env") {
        embedding_config.insert("api_key_env".to_string(), env_name);
    }
    if let Some(scheme) = provider_config_value(config, "embedding_auth_scheme") {
        embedding_config.insert("auth_scheme".to_string(), scheme);
    } else {
        embedding_config.insert("auth_scheme".to_string(), "bearer".to_string());
    }

    let shape = provider_config_value(config, "embedding_shape")
        .unwrap_or_else(|| "openai".to_string())
        .to_ascii_lowercase();
    let body = if shape == "text" {
        serde_json::json!({"text": text})
    } else {
        let model = provider_config_value(config, "embedding_model")
            .unwrap_or_else(|| "text-embedding-3-small".to_string());
        serde_json::json!({"input": text, "model": model})
    };
    let response = provider_json_request(
        &embedding_config,
        "bearer",
        reqwest::Method::POST,
        &embedding_url,
        Some(body),
    )
    .await?;
    extract_embedding(response)
}

pub(super) async fn custom_http_request(
    config: &std::collections::HashMap<String, String>,
    method: reqwest::Method,
    url: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut request = shared_http_client()
        .request(method, url)
        .timeout(std::time::Duration::from_secs(15));
    if let Some(token) = provider_token(config) {
        request = request.bearer_auth(token);
    }
    if let Some(headers) = config.get("headers_json") {
        let parsed: serde_json::Map<String, serde_json::Value> = serde_json::from_str(headers)
            .map_err(|error| format!("invalid headers_json: {error}"))?;
        for (key, value) in parsed {
            if let Some(value) = value.as_str() {
                request = request.header(key, value);
            }
        }
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    request
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())
}
