use super::*;

fn apply_configured_headers(
    mut request: reqwest::RequestBuilder,
    config: &std::collections::HashMap<String, String>,
) -> Result<reqwest::RequestBuilder, String> {
    let Some(raw) = config.get("headers_json") else {
        return Ok(request);
    };
    if raw.len() > 64 * 1024 {
        return Err("headers_json exceeds the 64 KiB limit".to_string());
    }
    let parsed: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(raw).map_err(|error| format!("invalid headers_json: {error}"))?;
    if parsed.len() > 64 {
        return Err("headers_json exceeds the 64-header limit".to_string());
    }
    for (key, value) in parsed {
        let value = value
            .as_str()
            .ok_or_else(|| format!("header '{key}' must be a string"))?;
        if value.len() > 8 * 1024 {
            return Err(format!("header '{key}' exceeds the 8 KiB limit"));
        }
        let name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| format!("header name '{key}' is invalid"))?;
        if matches!(
            name.as_str(),
            "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "proxy-authorization"
                | "proxy-authenticate"
                | "upgrade"
        ) {
            return Err(format!("header '{key}' is not allowed"));
        }
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| format!("header '{key}' has an invalid value"))?;
        request = request.header(name, value);
    }
    Ok(request)
}

#[async_trait]
impl MemoryProvider for CustomHttpProvider {
    fn name(&self) -> &'static str {
        "custom_http"
    }

    fn supports_strict_subject_scoping(&self) -> bool {
        true
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
    let url = validated_provider_request_url(url)?;
    let mut request = apply_provider_auth(
        shared_http_client()?
            .request(method, url)
            .timeout(std::time::Duration::from_secs(15)),
        config,
        default_auth_scheme,
    );
    request = apply_configured_headers(request, config)?;
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    provider_json_response(response).await
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
    let url = validated_provider_request_url(url)?;
    let mut request = shared_http_client()?
        .request(method, url)
        .timeout(std::time::Duration::from_secs(15));
    if let Some(token) = provider_token(config) {
        request = request.bearer_auth(token);
    }
    request = apply_configured_headers(request, config)?;
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    provider_json_response(response).await
}
