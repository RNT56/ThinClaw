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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = apply_provider_auth(client.request(method, url), config, default_auth_scheme);
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

pub(super) fn payload_text(payload: &serde_json::Value) -> String {
    if let Some(value) = payload.as_str() {
        return value.to_string();
    }
    for key in ["content", "text", "summary", "memory", "user_message"] {
        if let Some(value) = payload.get(key).and_then(|value| value.as_str())
            && !value.trim().is_empty()
        {
            return value.to_string();
        }
    }
    let user = payload
        .get("user")
        .or_else(|| payload.get("user_message"))
        .and_then(|value| value.as_str());
    let assistant = payload
        .get("assistant")
        .or_else(|| payload.get("assistant_response"))
        .and_then(|value| value.as_str());
    match (user, assistant) {
        (Some(user), Some(assistant)) => {
            format!("User: {user}\nAssistant: {assistant}")
        }
        _ => serde_json::to_string(payload).unwrap_or_else(|_| format!("{payload:?}")),
    }
}

pub(super) fn provider_memory_text_at_depth(
    value: &serde_json::Value,
    depth: usize,
) -> Option<String> {
    if let Some(text) = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }
    if depth > 2 {
        return None;
    }
    for key in [
        "summary",
        "memory",
        "text",
        "content",
        "document",
        "page_content",
        "value",
    ] {
        if let Some(text) = value
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
    }
    for key in ["payload", "metadata", "data", "record"] {
        if let Some(nested) = value.get(key)
            && let Some(text) = provider_memory_text_at_depth(nested, depth + 1)
        {
            return Some(text);
        }
    }
    None
}

pub(super) fn provider_score(value: &serde_json::Value) -> Option<f64> {
    for key in ["score", "similarity", "relevance", "rrf_score"] {
        if let Some(score) = value.get(key).and_then(|value| value.as_f64()) {
            return Some(score);
        }
    }
    value
        .get("metadata")
        .and_then(provider_score)
        .or_else(|| value.get("payload").and_then(provider_score))
}

pub(super) fn parse_matrix_hits(
    value: &serde_json::Value,
    provider: &str,
) -> Vec<ProviderMemoryHit> {
    let Some(document_batches) = value.get("documents").and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    let scores = value
        .get("scores")
        .or_else(|| value.get("distances"))
        .and_then(|value| value.as_array());
    let ids = value.get("ids").and_then(|value| value.as_array());
    let metadatas = value.get("metadatas").and_then(|value| value.as_array());
    let mut hits = Vec::new();
    for (batch_index, batch) in document_batches.iter().enumerate() {
        let Some(documents) = batch.as_array() else {
            continue;
        };
        let score_batch = scores
            .and_then(|batches| batches.get(batch_index))
            .and_then(|batch| batch.as_array());
        let id_batch = ids
            .and_then(|batches| batches.get(batch_index))
            .and_then(|batch| batch.as_array());
        let metadata_batch = metadatas
            .and_then(|batches| batches.get(batch_index))
            .and_then(|batch| batch.as_array());
        for (index, document) in documents.iter().enumerate() {
            let Some(summary) = document
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
            else {
                continue;
            };
            let provenance = serde_json::json!({
                "id": id_batch.and_then(|values| values.get(index)).cloned(),
                "metadata": metadata_batch.and_then(|values| values.get(index)).cloned(),
            });
            hits.push(ProviderMemoryHit {
                provider: provider.to_string(),
                summary,
                score: score_batch
                    .and_then(|values| values.get(index))
                    .and_then(|value| value.as_f64()),
                provenance,
            });
        }
    }
    hits
}

pub(in crate::agent::learning) fn parse_provider_hits(
    value: serde_json::Value,
    provider: &str,
) -> Vec<ProviderMemoryHit> {
    let matrix_hits = parse_matrix_hits(&value, provider);
    if !matrix_hits.is_empty() {
        return matrix_hits;
    }

    let point_items = value
        .get("result")
        .and_then(|value| value.get("points"))
        .and_then(|value| value.as_array())
        .cloned();
    let items = point_items
        .or_else(|| value.as_array().cloned())
        .or_else(|| {
            value
                .get("results")
                .and_then(|value| value.as_array())
                .cloned()
        })
        .or_else(|| {
            value
                .get("memories")
                .and_then(|value| value.as_array())
                .cloned()
        })
        .or_else(|| {
            value
                .get("data")
                .and_then(|value| value.as_array())
                .cloned()
        })
        .or_else(|| {
            value
                .get("result")
                .and_then(|value| value.as_array())
                .cloned()
        })
        .unwrap_or_default();

    items
        .into_iter()
        .filter_map(|item| {
            let summary = provider_memory_text_at_depth(&item, 0)?;
            Some(ProviderMemoryHit {
                provider: provider.to_string(),
                summary,
                score: provider_score(&item),
                provenance: item,
            })
        })
        .collect()
}

pub(super) fn extract_embedding(value: serde_json::Value) -> Result<Vec<f64>, String> {
    fn parse_vec(value: &serde_json::Value) -> Option<Vec<f64>> {
        let array = value.as_array()?;
        let mut out = Vec::with_capacity(array.len());
        for item in array {
            out.push(item.as_f64()?);
        }
        Some(out)
    }

    if let Some(embedding) = value.get("embedding").and_then(parse_vec) {
        return Ok(embedding);
    }
    if let Some(embedding) = value.get("vector").and_then(parse_vec) {
        return Ok(embedding);
    }
    if let Some(embedding) = value
        .get("data")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("embedding"))
        .and_then(parse_vec)
    {
        return Ok(embedding);
    }
    if let Some(embedding) = parse_vec(&value) {
        return Ok(embedding);
    }
    Err("embedding response did not contain an embedding vector".to_string())
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

pub(super) fn provider_export_messages(
    config: &std::collections::HashMap<String, String>,
    payload: &serde_json::Value,
) -> Vec<serde_json::Value> {
    if let Some(messages) = payload.get("messages").and_then(|value| value.as_array()) {
        return messages.clone();
    }
    vec![serde_json::json!({
        "role": provider_config_value(config, "export_role").unwrap_or_else(|| "user".to_string()),
        "content": payload_text(payload),
    })]
}

pub(super) async fn custom_http_request(
    config: &std::collections::HashMap<String, String>,
    method: reqwest::Method,
    url: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.request(method, url);
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

pub(in crate::agent::learning) fn parse_custom_http_hits(
    value: serde_json::Value,
    provider: &str,
) -> Vec<ProviderMemoryHit> {
    let items = value
        .as_array()
        .cloned()
        .or_else(|| {
            value
                .get("memories")
                .and_then(|value| value.as_array())
                .cloned()
        })
        .or_else(|| {
            value
                .get("results")
                .and_then(|value| value.as_array())
                .cloned()
        })
        .unwrap_or_default();
    items
        .into_iter()
        .filter_map(|item| {
            let summary = item
                .get("summary")
                .or_else(|| item.get("text"))
                .or_else(|| item.get("content"))
                .and_then(|value| value.as_str())?
                .trim()
                .to_string();
            if summary.is_empty() {
                return None;
            }
            Some(ProviderMemoryHit {
                provider: provider.to_string(),
                summary,
                score: item.get("score").and_then(|value| value.as_f64()),
                provenance: item,
            })
        })
        .collect()
}
