//! Provider-specific model metadata ingestion and local catalog refresh.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use serde::Deserialize;

use thinclaw_config::model_compat::{
    MODEL_CATALOG_VERSION, ModelCatalogSnapshot, ModelCompat, normalize_lookup_id,
};

/// Sync options for refreshing the local model compat DB.
#[derive(Debug, Clone)]
pub struct ModelMetadataSyncOptions {
    pub providers: Vec<String>,
    pub timeout: Duration,
    /// Provider slug -> API key/token.
    pub credentials: HashMap<String, String>,
}

/// Per-provider sync report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderSyncReport {
    pub provider: String,
    pub source_url: Option<String>,
    pub upserted: usize,
    pub unresolved: Vec<String>,
    pub error: Option<String>,
}

/// Result of a refresh operation.
#[derive(Debug, Clone)]
pub struct ModelMetadataSyncResult {
    pub snapshot: ModelCatalogSnapshot,
    pub reports: Vec<ProviderSyncReport>,
}

#[derive(Debug, Default)]
struct ProviderFetchOutcome {
    records: Vec<ModelCompat>,
    unresolved: Vec<String>,
    source_url: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct ExistingProviderIndex {
    by_exact: HashMap<String, ModelCompat>,
    by_normalized: HashMap<String, ModelCompat>,
}

impl ExistingProviderIndex {
    fn new(existing: &[ModelCompat], provider: &str) -> Self {
        let mut by_exact = HashMap::new();
        let mut by_normalized = HashMap::new();
        for model in existing.iter().filter(|model| model.provider == provider) {
            by_exact.insert(model.model_id.clone(), model.clone());
            by_normalized.insert(normalize_lookup_id(&model.model_id), model.clone());
        }
        Self {
            by_exact,
            by_normalized,
        }
    }

    fn find(&self, model_id: &str) -> Option<ModelCompat> {
        self.by_exact.get(model_id).cloned().or_else(|| {
            self.by_normalized
                .get(&normalize_lookup_id(model_id))
                .cloned()
        })
    }
}

/// Providers with first-class ingesters today.
pub fn supported_sync_providers() -> Vec<String> {
    vec![
        "openai".to_string(),
        "anthropic".to_string(),
        "deepseek".to_string(),
        "xai".to_string(),
        "moonshot".to_string(),
        "openrouter".to_string(),
    ]
}

/// Refresh the model compat catalog with provider-specific ingesters.
pub async fn refresh_model_catalog(
    existing: &[ModelCompat],
    options: &ModelMetadataSyncOptions,
) -> ModelMetadataSyncResult {
    let providers = if options.providers.is_empty() {
        supported_sync_providers()
    } else {
        options.providers.clone()
    };

    let client = match reqwest::Client::builder().timeout(options.timeout).build() {
        Ok(client) => client,
        Err(error) => {
            return ModelMetadataSyncResult {
                snapshot: ModelCatalogSnapshot {
                    version: MODEL_CATALOG_VERSION,
                    generated_at: Some(chrono::Utc::now().to_rfc3339()),
                    models: existing.to_vec(),
                },
                reports: vec![ProviderSyncReport {
                    provider: "client".to_string(),
                    source_url: None,
                    upserted: 0,
                    unresolved: Vec::new(),
                    error: Some(format!("failed to build HTTP client: {error}")),
                }],
            };
        }
    };

    let mut catalog: BTreeMap<(String, String), ModelCompat> = existing
        .iter()
        .cloned()
        .map(|model| ((model.provider.clone(), model.model_id.clone()), model))
        .collect();
    let mut reports = Vec::new();

    for provider in providers {
        let outcome = fetch_provider(
            &client,
            &provider,
            existing,
            options.credentials.get(&provider).map(String::as_str),
        )
        .await;

        let mut upserted = 0;
        for record in outcome.records {
            let key = (record.provider.clone(), record.model_id.clone());
            if catalog.get(&key) != Some(&record) {
                upserted += 1;
            }
            catalog.insert(key, record);
        }

        reports.push(ProviderSyncReport {
            provider,
            source_url: outcome.source_url,
            upserted,
            unresolved: outcome.unresolved,
            error: outcome.error,
        });
    }

    let generated_at = chrono::Utc::now().to_rfc3339();
    let mut models: Vec<ModelCompat> = catalog.into_values().collect();
    models.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.alias_of.is_some().cmp(&right.alias_of.is_some()))
            .then_with(|| left.model_id.cmp(&right.model_id))
    });

    ModelMetadataSyncResult {
        snapshot: ModelCatalogSnapshot {
            version: MODEL_CATALOG_VERSION,
            generated_at: Some(generated_at),
            models,
        },
        reports,
    }
}

async fn fetch_provider(
    client: &reqwest::Client,
    provider: &str,
    existing: &[ModelCompat],
    credential: Option<&str>,
) -> ProviderFetchOutcome {
    match provider {
        "openrouter" => fetch_openrouter_models(client).await,
        "moonshot" => fetch_moonshot_models(client, existing, credential).await,
        "xai" => fetch_xai_models(client, existing, credential).await,
        "openai" => {
            fetch_presence_sync_openai_compatible(
                client,
                provider,
                "https://api.openai.com/v1",
                "https://platform.openai.com/docs/api-reference/models/list",
                existing,
                credential,
                false,
            )
            .await
        }
        "deepseek" => {
            fetch_presence_sync_openai_compatible(
                client,
                provider,
                "https://api.deepseek.com/v1",
                "https://api-docs.deepseek.com/api/list-models",
                existing,
                credential,
                false,
            )
            .await
        }
        "anthropic" => fetch_presence_sync_anthropic(client, existing, credential).await,
        other => ProviderFetchOutcome {
            error: Some(format!(
                "no provider-specific model ingester is implemented for '{other}'"
            )),
            ..ProviderFetchOutcome::default()
        },
    }
}

async fn fetch_presence_sync_openai_compatible(
    _client: &reqwest::Client,
    provider: &str,
    base_url: &str,
    source_url: &str,
    existing: &[ModelCompat],
    credential: Option<&str>,
    include_context_from_response: bool,
) -> ProviderFetchOutcome {
    let Some(token) = credential else {
        return ProviderFetchOutcome {
            source_url: Some(source_url.to_string()),
            error: Some("credentials not configured".to_string()),
            ..ProviderFetchOutcome::default()
        };
    };

    let discovery = crate::discovery::ModelDiscovery::with_timeout(Duration::from_secs(15));
    let result = discovery
        .discover_openai_compatible(base_url, Some(&format!("Bearer {token}")))
        .await;
    if let Some(error) = result.error {
        return ProviderFetchOutcome {
            source_url: Some(source_url.to_string()),
            error: Some(error),
            ..ProviderFetchOutcome::default()
        };
    }

    let index = ExistingProviderIndex::new(existing, provider);
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    let mut unresolved = Vec::new();

    for discovered in result.models {
        if !seen.insert(discovered.id.clone()) {
            continue;
        }
        if let Some(mut model) = index.find(&discovered.id) {
            model.fetched_at = Some(fetched_at.clone());
            model.source_url = Some(source_url.to_string());
            if include_context_from_response && let Some(context_length) = discovered.context_length
            {
                model.context_window = context_length;
            }
            records.push(model);
        } else {
            unresolved.push(discovered.id);
        }
    }

    ProviderFetchOutcome {
        records,
        unresolved,
        source_url: Some(source_url.to_string()),
        error: None,
    }
}

async fn fetch_presence_sync_anthropic(
    _client: &reqwest::Client,
    existing: &[ModelCompat],
    credential: Option<&str>,
) -> ProviderFetchOutcome {
    let Some(api_key) = credential else {
        return ProviderFetchOutcome {
            source_url: Some("https://docs.anthropic.com/en/api/models-list".to_string()),
            error: Some("credentials not configured".to_string()),
            ..ProviderFetchOutcome::default()
        };
    };

    let discovery = crate::discovery::ModelDiscovery::with_timeout(Duration::from_secs(15));
    let result = discovery.discover_anthropic(api_key).await;
    if let Some(error) = result.error {
        return ProviderFetchOutcome {
            source_url: Some("https://docs.anthropic.com/en/api/models-list".to_string()),
            error: Some(error),
            ..ProviderFetchOutcome::default()
        };
    }

    let index = ExistingProviderIndex::new(existing, "anthropic");
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    let mut unresolved = Vec::new();

    for discovered in result.models {
        if !seen.insert(discovered.id.clone()) {
            continue;
        }
        if let Some(mut model) = index.find(&discovered.id) {
            model.fetched_at = Some(fetched_at.clone());
            model.source_url = Some("https://docs.anthropic.com/en/api/models-list".to_string());
            records.push(model);
        } else {
            unresolved.push(discovered.id);
        }
    }

    ProviderFetchOutcome {
        records,
        unresolved,
        source_url: Some("https://docs.anthropic.com/en/api/models-list".to_string()),
        error: None,
    }
}

async fn fetch_moonshot_models(
    client: &reqwest::Client,
    existing: &[ModelCompat],
    credential: Option<&str>,
) -> ProviderFetchOutcome {
    let Some(token) = credential else {
        return ProviderFetchOutcome {
            source_url: Some("https://platform.kimi.ai/docs/api/list-models".to_string()),
            error: Some("credentials not configured".to_string()),
            ..ProviderFetchOutcome::default()
        };
    };

    let response = match client
        .get("https://api.moonshot.ai/v1/models")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return ProviderFetchOutcome {
                source_url: Some("https://platform.kimi.ai/docs/api/list-models".to_string()),
                error: Some(format!("moonshot model list request failed: {error}")),
                ..ProviderFetchOutcome::default()
            };
        }
    };

    if !response.status().is_success() {
        return ProviderFetchOutcome {
            source_url: Some("https://platform.kimi.ai/docs/api/list-models".to_string()),
            error: Some(format!(
                "moonshot model list returned HTTP {}",
                response.status()
            )),
            ..ProviderFetchOutcome::default()
        };
    }

    let body: MoonshotModelsResponse = match response.json().await {
        Ok(body) => body,
        Err(error) => {
            return ProviderFetchOutcome {
                source_url: Some("https://platform.kimi.ai/docs/api/list-models".to_string()),
                error: Some(format!("failed to parse moonshot model list: {error}")),
                ..ProviderFetchOutcome::default()
            };
        }
    };

    let index = ExistingProviderIndex::new(existing, "moonshot");
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let mut records = Vec::new();
    let mut unresolved = Vec::new();

    for entry in body.data {
        if let Some(mut model) = index.find(&entry.id) {
            model.context_window = entry.context_length.unwrap_or(model.context_window);
            model.supports_vision = entry.supports_image_in || entry.supports_video_in;
            model.supports_thinking = entry.supports_reasoning;
            model.fetched_at = Some(fetched_at.clone());
            model.source_url = Some("https://platform.kimi.ai/docs/api/list-models".to_string());
            records.push(model);
        } else {
            unresolved.push(entry.id);
        }
    }

    ProviderFetchOutcome {
        records,
        unresolved,
        source_url: Some("https://platform.kimi.ai/docs/api/list-models".to_string()),
        error: None,
    }
}

async fn fetch_xai_models(
    client: &reqwest::Client,
    existing: &[ModelCompat],
    credential: Option<&str>,
) -> ProviderFetchOutcome {
    let Some(token) = credential else {
        return ProviderFetchOutcome {
            source_url: Some(
                "https://docs.x.ai/developers/rest-api-reference/inference/models".to_string(),
            ),
            error: Some("credentials not configured".to_string()),
            ..ProviderFetchOutcome::default()
        };
    };

    let response = match client
        .get("https://api.x.ai/v1/language-models")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return ProviderFetchOutcome {
                source_url: Some(
                    "https://docs.x.ai/developers/rest-api-reference/inference/models".to_string(),
                ),
                error: Some(format!("xAI language models request failed: {error}")),
                ..ProviderFetchOutcome::default()
            };
        }
    };

    if !response.status().is_success() {
        return ProviderFetchOutcome {
            source_url: Some(
                "https://docs.x.ai/developers/rest-api-reference/inference/models".to_string(),
            ),
            error: Some(format!(
                "xAI language models returned HTTP {}",
                response.status()
            )),
            ..ProviderFetchOutcome::default()
        };
    }

    let body: XaiLanguageModelsResponse = match response.json().await {
        Ok(body) => body,
        Err(error) => {
            return ProviderFetchOutcome {
                source_url: Some(
                    "https://docs.x.ai/developers/rest-api-reference/inference/models".to_string(),
                ),
                error: Some(format!("failed to parse xAI language models: {error}")),
                ..ProviderFetchOutcome::default()
            };
        }
    };

    let index = ExistingProviderIndex::new(existing, "xai");
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let mut records = Vec::new();
    let mut unresolved = Vec::new();

    for model in body.models {
        let Some(base) = index.find(&model.id) else {
            unresolved.push(model.id.clone());
            continue;
        };
        let mut updated = base.clone();
        updated.supports_vision =
            supports_visual_input(&model.input_modalities) || base.supports_vision;
        updated.pricing_input =
            xai_price_to_per_m(model.prompt_text_token_price).or(base.pricing_input);
        updated.pricing_output =
            xai_price_to_per_m(model.completion_text_token_price).or(base.pricing_output);
        updated.source_url =
            Some("https://docs.x.ai/developers/rest-api-reference/inference/models".to_string());
        updated.fetched_at = Some(fetched_at.clone());
        if let Some(version) = model.version {
            updated.capabilities.insert("version".to_string(), version);
        }
        if let Some(fingerprint) = model.fingerprint {
            updated
                .capabilities
                .insert("fingerprint".to_string(), fingerprint);
        }
        records.push(updated.clone());

        for alias in model.aliases.into_iter().filter(|alias| alias != &model.id) {
            let mut alias_record = updated.clone();
            alias_record.model_id = alias;
            alias_record.alias_of = Some(updated.model_id.clone());
            alias_record.display_name = alias_record.model_id.clone();
            records.push(alias_record);
        }
    }

    ProviderFetchOutcome {
        records,
        unresolved,
        source_url: Some(
            "https://docs.x.ai/developers/rest-api-reference/inference/models".to_string(),
        ),
        error: None,
    }
}

async fn fetch_openrouter_models(client: &reqwest::Client) -> ProviderFetchOutcome {
    let response = match client
        .get("https://openrouter.ai/api/v1/models")
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return ProviderFetchOutcome {
                source_url: Some("https://openrouter.ai/api/v1/models".to_string()),
                error: Some(format!("openrouter model list request failed: {error}")),
                ..ProviderFetchOutcome::default()
            };
        }
    };

    if !response.status().is_success() {
        return ProviderFetchOutcome {
            source_url: Some("https://openrouter.ai/api/v1/models".to_string()),
            error: Some(format!(
                "openrouter model list returned HTTP {}",
                response.status()
            )),
            ..ProviderFetchOutcome::default()
        };
    }

    let body: OpenRouterModelsResponse = match response.json().await {
        Ok(body) => body,
        Err(error) => {
            return ProviderFetchOutcome {
                source_url: Some("https://openrouter.ai/api/v1/models".to_string()),
                error: Some(format!("failed to parse openrouter model list: {error}")),
                ..ProviderFetchOutcome::default()
            };
        }
    };

    let fetched_at = chrono::Utc::now().to_rfc3339();
    let mut records = Vec::new();
    let mut unresolved = Vec::new();

    for model in body.data {
        match openrouter_record_from_model(&model, &fetched_at) {
            Some(record) => records.push(record),
            None => unresolved.push(model.id),
        }
    }

    ProviderFetchOutcome {
        records,
        unresolved,
        source_url: Some("https://openrouter.ai/api/v1/models".to_string()),
        error: None,
    }
}

fn openrouter_record_from_model(model: &OpenRouterModel, fetched_at: &str) -> Option<ModelCompat> {
    let context_window = model
        .top_provider
        .as_ref()
        .and_then(|provider| provider.context_length)
        .or(model.context_length)?;
    let max_output_tokens = model
        .top_provider
        .as_ref()
        .and_then(|provider| provider.max_completion_tokens)?;

    let input_modalities = model
        .architecture
        .as_ref()
        .map(|architecture| architecture.input_modalities.clone())
        .unwrap_or_default();
    let output_modalities = model
        .architecture
        .as_ref()
        .map(|architecture| architecture.output_modalities.clone())
        .unwrap_or_default();
    let supported_parameters = model.supported_parameters.clone().unwrap_or_default();

    let mut capabilities = HashMap::new();
    if let Some(canonical_slug) = &model.canonical_slug {
        capabilities.insert("canonical_slug".to_string(), canonical_slug.clone());
    }
    if let Some(details_path) = model.links.as_ref().and_then(|links| links.details.clone()) {
        capabilities.insert("details_path".to_string(), details_path);
    }
    if !input_modalities.is_empty() {
        capabilities.insert("input_modalities".to_string(), input_modalities.join(","));
    }
    if !output_modalities.is_empty() {
        capabilities.insert("output_modalities".to_string(), output_modalities.join(","));
    }

    Some(ModelCompat {
        provider: "openrouter".to_string(),
        model_id: model.id.clone(),
        alias_of: normalize_openrouter_alias(&model.id),
        display_name: model.name.clone().unwrap_or_else(|| model.id.clone()),
        context_window,
        max_output_tokens,
        supports_tools: supports_parameter(&supported_parameters, "tools")
            || supports_parameter(&supported_parameters, "tool_choice"),
        supports_vision: supports_visual_input(&input_modalities),
        supports_streaming: true,
        supports_thinking: supports_parameter(&supported_parameters, "reasoning")
            || supports_parameter(&supported_parameters, "reasoning_effort")
            || supports_parameter(&supported_parameters, "include_reasoning"),
        supports_json_mode: supports_parameter(&supported_parameters, "response_format")
            || supports_parameter(&supported_parameters, "structured_outputs"),
        supports_system_prompt: true,
        pricing_input: model
            .pricing
            .as_ref()
            .and_then(|pricing| parse_openrouter_price_per_m(pricing.prompt.as_deref())),
        pricing_output: model
            .pricing
            .as_ref()
            .and_then(|pricing| parse_openrouter_price_per_m(pricing.completion.as_deref())),
        source_url: Some("https://openrouter.ai/api/v1/models".to_string()),
        fetched_at: Some(fetched_at.to_string()),
        capabilities,
    })
}

fn normalize_openrouter_alias(model_id: &str) -> Option<String> {
    let stripped = model_id.strip_prefix('~').unwrap_or(model_id);
    stripped
        .split_once('/')
        .map(|(_, base_model)| base_model.to_string())
}

fn supports_parameter(parameters: &[String], name: &str) -> bool {
    parameters.iter().any(|parameter| parameter == name)
}

fn supports_visual_input(modalities: &[String]) -> bool {
    modalities
        .iter()
        .any(|modality| matches!(modality.as_str(), "image" | "video"))
}

fn parse_openrouter_price_per_m(raw: Option<&str>) -> Option<f64> {
    let value = raw?.parse::<f64>().ok()?;
    if value <= 0.0 {
        return None;
    }
    Some(value * 1_000_000.0)
}

fn xai_price_to_per_m(raw: Option<i64>) -> Option<f64> {
    raw.map(|value| value as f64 / 10_000.0)
}

#[derive(Debug, Deserialize)]
struct MoonshotModelsResponse {
    data: Vec<MoonshotModelEntry>,
}

#[derive(Debug, Deserialize)]
struct MoonshotModelEntry {
    id: String,
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    supports_image_in: bool,
    #[serde(default)]
    supports_video_in: bool,
    #[serde(default)]
    supports_reasoning: bool,
}

#[derive(Debug, Deserialize)]
struct XaiLanguageModelsResponse {
    models: Vec<XaiLanguageModel>,
}

#[derive(Debug, Deserialize)]
struct XaiLanguageModel {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    prompt_text_token_price: Option<i64>,
    #[serde(default)]
    completion_text_token_price: Option<i64>,
    #[serde(default)]
    fingerprint: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    #[serde(default)]
    canonical_slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    architecture: Option<OpenRouterArchitecture>,
    #[serde(default)]
    pricing: Option<OpenRouterPricing>,
    #[serde(default)]
    top_provider: Option<OpenRouterTopProvider>,
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
    #[serde(default)]
    links: Option<OpenRouterLinks>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    output_modalities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterTopProvider {
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    max_completion_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterLinks {
    #[serde(default)]
    details: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrouter_alias_strips_provider_prefix() {
        assert_eq!(
            normalize_openrouter_alias("anthropic/claude-sonnet-4-20250514").as_deref(),
            Some("claude-sonnet-4-20250514")
        );
        assert_eq!(
            normalize_openrouter_alias("~anthropic/claude-opus-latest").as_deref(),
            Some("claude-opus-latest")
        );
    }

    #[test]
    fn openrouter_model_hydrates_rich_metadata() {
        let record = openrouter_record_from_model(
            &serde_json::from_str::<OpenRouterModel>(
                r#"{
                    "id": "openai/gpt-5.4",
                    "canonical_slug": "openai/gpt-5.4-20260421",
                    "name": "OpenAI: GPT-5.4",
                    "context_length": 400000,
                    "architecture": {
                        "input_modalities": ["text", "image"],
                        "output_modalities": ["text"]
                    },
                    "pricing": {
                        "prompt": "0.0000025",
                        "completion": "0.000015"
                    },
                    "top_provider": {
                        "context_length": 400000,
                        "max_completion_tokens": 128000
                    },
                    "supported_parameters": [
                        "max_tokens",
                        "response_format",
                        "reasoning",
                        "tool_choice",
                        "tools"
                    ],
                    "links": {
                        "details": "/api/v1/models/openai/gpt-5.4-20260421/endpoints"
                    }
                }"#,
            )
            .unwrap(),
            "2026-04-23T00:00:00Z",
        )
        .unwrap();

        assert_eq!(record.alias_of.as_deref(), Some("gpt-5.4"));
        assert_eq!(record.context_window, 400000);
        assert_eq!(record.max_output_tokens, 128000);
        assert!(record.supports_tools);
        assert!(record.supports_vision);
        assert!(record.supports_thinking);
        assert_eq!(record.pricing_input, Some(2.5));
        assert_eq!(record.pricing_output, Some(15.0));
    }

    #[test]
    fn xai_price_conversion_matches_doc_units() {
        assert_eq!(xai_price_to_per_m(Some(2500)), Some(0.25));
    }
}
