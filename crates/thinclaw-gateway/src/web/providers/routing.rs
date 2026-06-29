//! Provider model discovery, ranking/priority, and route simulation.

use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(serde::Deserialize)]
pub struct RouteSimulateRequest {
    pub prompt: String,
    #[serde(default)]
    pub has_vision: bool,
    #[serde(default)]
    pub has_tools: bool,
    #[serde(default)]
    pub requires_streaming: bool,
}

#[derive(serde::Serialize)]
pub struct RouteSimulateResponse {
    pub target: String,
    pub reason: String,
    #[serde(default)]
    pub fallback_chain: Vec<String>,
    #[serde(default)]
    pub candidate_list: Vec<String>,
    #[serde(default)]
    pub rejections: Vec<String>,
    #[serde(default)]
    pub score_breakdown: Vec<RouteSimulateScore>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct RouteSimulateScore {
    pub target: String,
    pub telemetry_key: Option<String>,
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
    pub policy_bias: f64,
    pub composite: f64,
}

#[derive(Debug, Clone)]
pub struct RouteSimulateResponseInput {
    pub target: String,
    pub reason: String,
    pub fallback_chain: Vec<String>,
    pub candidate_list: Vec<String>,
    pub rejections: Vec<String>,
    pub score_breakdown: Vec<RouteSimulateScoreInput>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RouteSimulateScoreInput {
    pub target: String,
    pub telemetry_key: Option<String>,
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
    pub policy_bias: f64,
    pub composite: f64,
}

pub fn route_simulate_response(input: RouteSimulateResponseInput) -> RouteSimulateResponse {
    RouteSimulateResponse {
        target: input.target,
        reason: input.reason,
        fallback_chain: input.fallback_chain,
        candidate_list: input.candidate_list,
        rejections: input.rejections,
        score_breakdown: input
            .score_breakdown
            .into_iter()
            .map(route_simulate_score)
            .collect(),
        diagnostics: input.diagnostics,
    }
}

pub fn route_simulate_score(input: RouteSimulateScoreInput) -> RouteSimulateScore {
    RouteSimulateScore {
        target: input.target,
        telemetry_key: input.telemetry_key,
        quality: input.quality,
        cost: input.cost,
        latency: input.latency,
        health: input.health,
        policy_bias: input.policy_bias,
        composite: input.composite,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderModelOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    pub source: String,
    pub recommended_primary: bool,
    pub recommended_cheap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredProviderModel {
    pub id: String,
    pub name: String,
    pub is_chat: bool,
    pub context_length: Option<u32>,
}

pub fn provider_model_options_from_discovery(
    slug: &str,
    default_model: &str,
    discovered: Vec<DiscoveredProviderModel>,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
    catalog_suggested_cheap_model: Option<&str>,
) -> (
    Vec<ProviderModelOption>,
    Option<String>,
    Option<String>,
    bool,
) {
    let mut discovered_map = BTreeMap::new();
    for model in discovered.into_iter().filter(|model| {
        if slug == "openai" {
            is_openai_chat_model(&model.id)
        } else {
            model.is_chat
        }
    }) {
        discovered_map.entry(model.id.clone()).or_insert(model);
    }

    let has_live_models = !discovered_map.is_empty();
    let current_primary_model =
        current_primary_model.filter(|model| discovered_map.contains_key(*model));
    let current_cheap_model =
        current_cheap_model.filter(|model| discovered_map.contains_key(*model));
    let preferred_default_model = (!default_model.is_empty()
        && discovered_map.contains_key(default_model))
    .then(|| default_model.to_string());
    let suggested_provider_cheap = catalog_suggested_cheap_model
        .map(str::to_string)
        .filter(|model| discovered_map.contains_key(model.as_str()));

    let suggested_primary_model = current_primary_model
        .map(str::to_string)
        .or_else(|| preferred_default_model.clone())
        .or_else(|| {
            discovered_map
                .keys()
                .max_by_key(|model| primary_model_rank(model))
                .cloned()
        })
        .or_else(|| {
            if has_live_models {
                None
            } else {
                Some(default_model.to_string())
            }
        });

    let suggested_cheap_model = current_cheap_model
        .map(str::to_string)
        .or(suggested_provider_cheap)
        .or_else(|| {
            discovered_map
                .keys()
                .max_by_key(|model| cheap_model_rank(model))
                .cloned()
        })
        .or_else(|| {
            if has_live_models {
                suggested_primary_model.clone()
            } else {
                catalog_suggested_cheap_model
                    .map(str::to_string)
                    .or_else(|| suggested_primary_model.clone())
            }
        });

    let mut model_ids = BTreeSet::new();
    let mut ordered_ids = Vec::new();
    for id in discovered_map.keys() {
        if model_ids.insert(id.clone()) {
            ordered_ids.push(id.clone());
        }
    }

    ordered_ids.sort_by(|a, b| {
        if matches!(slug, "openai" | "minimax" | "cohere") {
            let priority = |model: &String| match slug {
                "openai" => openai_model_priority(model),
                "minimax" => minimax_model_priority(model),
                "cohere" => cohere_model_priority(model),
                _ => usize::MAX,
            };
            priority(a).cmp(&priority(b))
        } else {
            model_display_rank(
                a,
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
                current_primary_model,
                current_cheap_model,
            )
            .cmp(&model_display_rank(
                b,
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
                current_primary_model,
                current_cheap_model,
            ))
            .reverse()
            .then_with(|| a.cmp(b))
        }
    });

    let models = ordered_ids
        .into_iter()
        .map(|id| {
            let discovered = discovered_map.get(&id);
            ProviderModelOption {
                id: id.clone(),
                label: discovered
                    .map(|model| model.name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| id.clone()),
                context_length: discovered.and_then(|model| model.context_length),
                source: if discovered.is_some() {
                    "discovered".to_string()
                } else {
                    "configured".to_string()
                },
                recommended_primary: suggested_primary_model.as_deref() == Some(id.as_str()),
                recommended_cheap: suggested_cheap_model.as_deref() == Some(id.as_str()),
            }
        })
        .collect();

    (
        models,
        suggested_primary_model,
        suggested_cheap_model,
        has_live_models,
    )
}

pub fn fallback_provider_model_options(
    default_model: &str,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
    suggested_primary_model: Option<&str>,
    suggested_cheap_model: Option<&str>,
    fallback_models: impl IntoIterator<Item = (String, String)>,
) -> Vec<ProviderModelOption> {
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();

    for id in [
        current_primary_model,
        current_cheap_model,
        suggested_primary_model,
        suggested_cheap_model,
        Some(default_model),
    ]
    .into_iter()
    .flatten()
    {
        if seen.insert(id.to_string()) {
            models.push(ProviderModelOption {
                id: id.to_string(),
                label: id.to_string(),
                context_length: None,
                source: if id == default_model {
                    "default".to_string()
                } else {
                    "configured".to_string()
                },
                recommended_primary: suggested_primary_model == Some(id),
                recommended_cheap: suggested_cheap_model == Some(id),
            });
        }
    }

    for (static_id, label) in fallback_models {
        if seen.insert(static_id.clone()) {
            models.push(ProviderModelOption {
                id: static_id,
                label,
                context_length: None,
                source: "curated".to_string(),
                recommended_primary: false,
                recommended_cheap: false,
            });
        }
    }

    if models.is_empty() && !default_model.is_empty() {
        models.push(ProviderModelOption {
            id: default_model.to_string(),
            label: default_model.to_string(),
            context_length: None,
            source: "default".to_string(),
            recommended_primary: true,
            recommended_cheap: suggested_cheap_model == Some(default_model),
        });
    }

    models
}

pub fn static_fallback_provider_models(slug: &str) -> Vec<(String, String)> {
    match slug {
        "anthropic" => vec![
            (
                "claude-opus-4-7".to_string(),
                "Claude Opus 4.7 (recommended)".to_string(),
            ),
            (
                "claude-opus-4-6".to_string(),
                "Claude Opus 4.6 (latest)".to_string(),
            ),
            (
                "claude-sonnet-4-6".to_string(),
                "Claude Sonnet 4.6".to_string(),
            ),
            ("claude-opus-4-5".to_string(), "Claude Opus 4.5".to_string()),
            (
                "claude-sonnet-4-5".to_string(),
                "Claude Sonnet 4.5".to_string(),
            ),
            (
                "claude-haiku-4-5".to_string(),
                "Claude Haiku 4.5 (fast)".to_string(),
            ),
        ],
        "openai" => vec![
            (
                "gpt-5.3-codex".to_string(),
                "GPT-5.3 Codex (latest)".to_string(),
            ),
            ("gpt-5.2-codex".to_string(), "GPT-5.2 Codex".to_string()),
            ("gpt-5.2".to_string(), "GPT-5.2".to_string()),
            (
                "gpt-5.1-codex-mini".to_string(),
                "GPT-5.1 Codex Mini (fast)".to_string(),
            ),
            ("gpt-5".to_string(), "GPT-5".to_string()),
            ("gpt-5-mini".to_string(), "GPT-5 Mini".to_string()),
            ("gpt-4.1".to_string(), "GPT-4.1".to_string()),
            ("gpt-4.1-mini".to_string(), "GPT-4.1 Mini".to_string()),
            (
                "o4-mini".to_string(),
                "o4-mini (fast reasoning)".to_string(),
            ),
            ("o3".to_string(), "o3 (reasoning)".to_string()),
        ],
        "gemini" => vec![
            ("gemini-2.5-pro".to_string(), "Gemini 2.5 Pro".to_string()),
            (
                "gemini-2.5-flash".to_string(),
                "Gemini 2.5 Flash".to_string(),
            ),
            (
                "gemini-2.5-flash-lite".to_string(),
                "Gemini 2.5 Flash Lite".to_string(),
            ),
        ],
        "groq" => vec![
            (
                "llama-3.3-70b-versatile".to_string(),
                "Llama 3.3 70B".to_string(),
            ),
            (
                "llama-3.1-8b-instant".to_string(),
                "Llama 3.1 8B Instant".to_string(),
            ),
        ],
        "mistral" => vec![
            (
                "mistral-large-latest".to_string(),
                "Mistral Large".to_string(),
            ),
            (
                "mistral-small-latest".to_string(),
                "Mistral Small".to_string(),
            ),
        ],
        "xai" => vec![
            ("grok-3".to_string(), "Grok 3".to_string()),
            ("grok-3-mini".to_string(), "Grok 3 Mini".to_string()),
        ],
        "deepseek" => vec![
            ("deepseek-chat".to_string(), "DeepSeek Chat".to_string()),
            (
                "deepseek-reasoner".to_string(),
                "DeepSeek Reasoner".to_string(),
            ),
        ],
        "openrouter" => vec![
            (
                "anthropic/claude-sonnet-4-20250514".to_string(),
                "Claude Sonnet 4 (via OR)".to_string(),
            ),
            (
                "openai/gpt-5.3-codex".to_string(),
                "GPT-5.3 Codex (via OR)".to_string(),
            ),
            (
                "google/gemini-2.5-flash".to_string(),
                "Gemini 2.5 Flash (via OR)".to_string(),
            ),
        ],
        "together" => vec![
            (
                "meta-llama/Llama-3.3-70B-Instruct-Turbo".to_string(),
                "Llama 3.3 70B Turbo".to_string(),
            ),
            (
                "meta-llama/Llama-3.1-8B-Instruct-Turbo".to_string(),
                "Llama 3.1 8B Turbo".to_string(),
            ),
        ],
        "cerebras" => vec![("llama-3.3-70b".to_string(), "Llama 3.3 70B".to_string())],
        "nvidia" => vec![(
            "meta/llama-3.3-70b-instruct".to_string(),
            "Llama 3.3 70B".to_string(),
        )],
        "minimax" => vec![
            ("MiniMax-M2.7".to_string(), "MiniMax M2.7".to_string()),
            ("MiniMax-M2.5".to_string(), "MiniMax M2.5".to_string()),
            (
                "MiniMax-M2.5-highspeed".to_string(),
                "MiniMax M2.5 Highspeed".to_string(),
            ),
            ("MiniMax-M2.1".to_string(), "MiniMax M2.1".to_string()),
            (
                "MiniMax-M2.1-highspeed".to_string(),
                "MiniMax M2.1 Highspeed".to_string(),
            ),
            ("MiniMax-M2".to_string(), "MiniMax M2".to_string()),
        ],
        "cohere" => vec![
            ("command-a-03-2025".to_string(), "Command A".to_string()),
            (
                "command-r-plus-08-2024".to_string(),
                "Command R+".to_string(),
            ),
            ("command-r-08-2024".to_string(), "Command R".to_string()),
            ("command-r7b-12-2024".to_string(), "Command R7B".to_string()),
        ],
        "tinfoil" => vec![("kimi-k2-5".to_string(), "Kimi K2.5".to_string())],
        _ => vec![],
    }
}

pub fn provider_fallback_model_catalog(
    slug: &str,
    dynamic_models: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let dynamic: Vec<_> = dynamic_models.into_iter().collect();
    if dynamic.is_empty() {
        static_fallback_provider_models(slug)
    } else {
        dynamic
    }
}

pub fn route_target_is_available_for_enabled_providers(
    target: &str,
    enabled: &HashSet<String>,
) -> bool {
    if matches!(target, "primary" | "cheap") {
        return true;
    }
    if let Some(slug) = target
        .strip_suffix("@primary")
        .or_else(|| target.strip_suffix("@cheap"))
    {
        return enabled.contains(slug);
    }
    if let Some((slug, _)) = target.split_once('/') {
        return enabled.contains(slug);
    }
    false
}

pub fn is_openai_chat_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    let is_chat_family = id.starts_with("gpt-")
        || id.starts_with("chatgpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("o5");
    let is_non_chat_variant = id.contains("realtime")
        || id.contains("audio")
        || id.contains("transcribe")
        || id.contains("tts")
        || id.contains("embedding")
        || id.contains("moderation")
        || id.contains("image");
    is_chat_family && !is_non_chat_variant
}

pub fn openai_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();
    const EXACT_PRIORITY: &[&str] = &[
        "gpt-5.3-codex",
        "gpt-5.2-codex",
        "gpt-5.2",
        "gpt-5.1-codex-mini",
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-nano",
        "o4-mini",
        "o3",
        "o1",
        "gpt-4.1",
        "gpt-4.1-mini",
        "gpt-4o",
        "gpt-4o-mini",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|model| id == *model) {
        return pos;
    }

    const PREFIX_PRIORITY: &[&str] = &[
        "gpt-5.", "gpt-5-", "o3-", "o4-", "o1-", "gpt-4.1-", "gpt-4o-", "gpt-3.5-", "chatgpt-",
    ];
    if let Some(pos) = PREFIX_PRIORITY
        .iter()
        .position(|prefix| id.starts_with(prefix))
    {
        return EXACT_PRIORITY.len() + pos;
    }

    EXACT_PRIORITY.len() + PREFIX_PRIORITY.len() + 1
}

pub fn minimax_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();
    const EXACT_PRIORITY: &[&str] = &[
        "minimax-m2.7",
        "minimax-m2.5",
        "minimax-m2.5-highspeed",
        "minimax-m2.1",
        "minimax-m2.1-highspeed",
        "minimax-m2",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|model| id == *model) {
        return pos;
    }
    if id.contains("m2.7") {
        return EXACT_PRIORITY.len();
    }
    if id.contains("m2.5") && !id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 1;
    }
    if id.contains("m2.5") && id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 2;
    }
    if id.contains("m2.1") && !id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 3;
    }
    if id.contains("m2.1") && id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 4;
    }
    if id.contains("m2") {
        return EXACT_PRIORITY.len() + 5;
    }
    EXACT_PRIORITY.len() + 50
}

pub fn cohere_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();
    const EXACT_PRIORITY: &[&str] = &[
        "command-a-03-2025",
        "command-r-plus-08-2024",
        "command-r-08-2024",
        "command-r7b-12-2024",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|model| id == *model) {
        return pos;
    }
    if id.starts_with("command-a") {
        return EXACT_PRIORITY.len();
    }
    if id.starts_with("command-r-plus") {
        return EXACT_PRIORITY.len() + 1;
    }
    if id.starts_with("command-r") {
        return EXACT_PRIORITY.len() + 2;
    }
    EXACT_PRIORITY.len() + 50
}

pub fn primary_model_rank(model: &str) -> i32 {
    let lower = model.to_lowercase();
    let mut score = 0;
    if lower.contains("pro")
        || lower.contains("sonnet")
        || lower.contains("opus")
        || lower.contains("command-a")
        || lower.contains("4o")
        || lower.contains("large")
        || lower.contains("70b")
    {
        score += 40;
    }
    if lower.contains("m2.7") {
        score += 52;
    } else if lower.contains("m2.5") && !lower.contains("highspeed") {
        score += 48;
    } else if lower.contains("m2.1") && !lower.contains("highspeed") {
        score += 44;
    } else if lower.contains("command-r-plus") {
        score += 34;
    }
    if lower.contains("mini")
        || lower.contains("haiku")
        || lower.contains("flash-lite")
        || lower.contains("nano")
        || lower.contains("small")
        || lower.contains("8b")
        || lower.contains("instant")
    {
        score -= 18;
    }
    if lower.contains("highspeed") || lower.contains("r7b") {
        score -= 14;
    }
    if lower.contains("embedding")
        || lower.contains("audio")
        || lower.contains("tts")
        || lower.contains("image")
        || lower.contains("moderation")
    {
        score -= 100;
    }
    score
}

pub fn cheap_model_rank(model: &str) -> i32 {
    let lower = model.to_lowercase();
    let mut score = 0;
    if lower.contains("mini")
        || lower.contains("haiku")
        || lower.contains("flash-lite")
        || lower.contains("flash")
        || lower.contains("nano")
        || lower.contains("small")
        || lower.contains("instant")
        || lower.contains("8b")
    {
        score += 45;
    }
    if lower.contains("highspeed") || lower.contains("r7b") {
        score += 42;
    }
    if lower.contains("pro")
        || lower.contains("opus")
        || lower.contains("sonnet")
        || lower.contains("command-a")
        || lower.contains("large")
        || lower.contains("70b")
    {
        score -= 18;
    }
    if lower.contains("embedding")
        || lower.contains("audio")
        || lower.contains("tts")
        || lower.contains("image")
        || lower.contains("moderation")
    {
        score -= 100;
    }
    score
}

pub fn model_display_rank(
    model: &str,
    suggested_primary_model: Option<&str>,
    suggested_cheap_model: Option<&str>,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
) -> i32 {
    let mut score = primary_model_rank(model).max(cheap_model_rank(model));
    if suggested_primary_model == Some(model) {
        score += 60;
    }
    if suggested_cheap_model == Some(model) {
        score += 50;
    }
    if current_primary_model == Some(model) {
        score += 40;
    }
    if current_cheap_model == Some(model) {
        score += 35;
    }
    score
}
