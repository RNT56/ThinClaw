//! Model compatibility catalog with disk + embedded fallback.
//!
//! The local compat DB is stored as JSON and can be refreshed by provider-
//! specific ingesters. This module exposes the normalized runtime view used by
//! routing, capability lookup, and UI surfaces.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Version of the on-disk model catalog schema.
pub const MODEL_CATALOG_VERSION: u32 = 1;

/// Full model compatibility descriptor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCompat {
    /// Provider slug (e.g., "openai", "anthropic", "moonshot").
    pub provider: String,
    /// Model identifier accepted by the provider.
    pub model_id: String,
    /// Optional canonical/snapshot model this record aliases.
    #[serde(default)]
    pub alias_of: Option<String>,
    /// Human-readable display name.
    #[serde(default)]
    pub display_name: String,
    /// Context window (tokens).
    pub context_window: u32,
    /// Max output tokens.
    pub max_output_tokens: u32,
    /// Whether the model supports tool use.
    pub supports_tools: bool,
    /// Whether the model supports vision (images).
    pub supports_vision: bool,
    /// Whether the model supports streaming.
    pub supports_streaming: bool,
    /// Whether the model supports extended thinking.
    pub supports_thinking: bool,
    /// Whether the model supports JSON mode.
    #[serde(default)]
    pub supports_json_mode: bool,
    /// Whether the model supports system prompts.
    #[serde(default)]
    pub supports_system_prompt: bool,
    /// Input price per million tokens (USD).
    #[serde(default, alias = "input_price_per_m")]
    pub pricing_input: Option<f64>,
    /// Output price per million tokens (USD).
    #[serde(default, alias = "output_price_per_m")]
    pub pricing_output: Option<f64>,
    /// Source URL where this record was hydrated or documented.
    #[serde(default)]
    pub source_url: Option<String>,
    /// RFC3339 timestamp indicating when this record was last refreshed.
    #[serde(default)]
    pub fetched_at: Option<String>,
    /// Additional provider-specific capabilities.
    #[serde(default)]
    pub capabilities: HashMap<String, String>,
}

/// Snapshot of the local compat DB.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCatalogSnapshot {
    #[serde(default = "default_catalog_version")]
    pub version: u32,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelCompat>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DiskCatalogFormat {
    Snapshot(ModelCatalogSnapshot),
    Models(Vec<ModelCompat>),
}

fn default_catalog_version() -> u32 {
    MODEL_CATALOG_VERSION
}

impl ModelCompat {
    /// Whether this model is a frontier (top-tier) model.
    pub fn is_frontier(&self) -> bool {
        self.context_window >= 100_000 && self.supports_tools && self.supports_streaming
    }

    /// Whether this model is "cheap" (under $1/M input).
    pub fn is_budget(&self) -> bool {
        self.pricing_input.map(|p| p < 1.0).unwrap_or(true)
    }

    /// Total cost estimate for a conversation (input + output tokens).
    pub fn estimate_cost(&self, input_tokens: u32, output_tokens: u32) -> Option<f64> {
        let input_price = self.pricing_input?;
        let output_price = self.pricing_output?;

        Some(
            (input_tokens as f64 / 1_000_000.0) * input_price
                + (output_tokens as f64 / 1_000_000.0) * output_price,
        )
    }

    /// Routing quality score derived from structured compatibility metadata.
    pub fn routing_quality_score(&self) -> f64 {
        estimate_routing_quality(
            Some(self.supports_streaming),
            Some(self.supports_tools),
            Some(self.supports_vision),
            Some(self.supports_thinking),
            Some(self.supports_json_mode),
            Some(self.supports_system_prompt),
            Some(self.context_window),
            Some(self.max_output_tokens),
            total_price_per_m(self.pricing_input, self.pricing_output),
        )
    }
}

/// Estimate a routing quality score from structured model metadata.
///
/// This returns a concrete score for every input shape so routing never falls
/// back to an "unknown/generic" bucket.
pub fn estimate_routing_quality(
    supports_streaming: Option<bool>,
    supports_tools: Option<bool>,
    supports_vision: Option<bool>,
    supports_thinking: Option<bool>,
    supports_json_mode: Option<bool>,
    supports_system_prompt: Option<bool>,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    total_price_per_m: Option<f64>,
) -> f64 {
    let mut score = 0.16;
    score += bool_signal(supports_streaming, 0.04, 0.02);
    score += bool_signal(supports_tools, 0.12, 0.03);
    score += bool_signal(supports_vision, 0.06, 0.01);
    score += bool_signal(supports_thinking, 0.14, 0.03);
    score += bool_signal(supports_json_mode, 0.05, 0.02);
    score += bool_signal(supports_system_prompt, 0.03, 0.02);
    score += normalized_log_range(
        context_window.map(|value| value as f64),
        8_000.0,
        1_000_000.0,
    ) * 0.18;
    score += normalized_log_range(
        max_output_tokens.map(|value| value as f64),
        2_048.0,
        128_000.0,
    ) * 0.10;
    score += normalized_log_range(total_price_per_m, 0.10, 40.0) * 0.10;
    score.clamp(0.05, 0.99)
}

fn bool_signal(value: Option<bool>, yes_weight: f64, unknown_weight: f64) -> f64 {
    match value {
        Some(true) => yes_weight,
        Some(false) => 0.0,
        None => unknown_weight,
    }
}

fn normalized_log_range(value: Option<f64>, min: f64, max: f64) -> f64 {
    let Some(value) = value.filter(|value| *value > 0.0) else {
        return 0.5;
    };
    let clamped = value.clamp(min, max);
    let min_ln = min.ln();
    let max_ln = max.ln();
    if (max_ln - min_ln).abs() < f64::EPSILON {
        return 1.0;
    }
    ((clamped.ln() - min_ln) / (max_ln - min_ln)).clamp(0.0, 1.0)
}

fn total_price_per_m(pricing_input: Option<f64>, pricing_output: Option<f64>) -> Option<f64> {
    match (pricing_input, pricing_output) {
        (Some(input), Some(output)) => Some(input + output),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    }
}

fn sanitize_model(mut model: ModelCompat) -> ModelCompat {
    if model.display_name.trim().is_empty() {
        model.display_name = model.model_id.clone();
    }
    model.provider = model.provider.trim().to_string();
    model.model_id = model.model_id.trim().to_string();
    if let Some(alias_of) = model.alias_of.as_mut() {
        *alias_of = alias_of.trim().to_string();
        if alias_of.is_empty() {
            model.alias_of = None;
        }
    }
    model
}

fn sanitize_snapshot(mut snapshot: ModelCatalogSnapshot) -> ModelCatalogSnapshot {
    snapshot.version = snapshot.version.max(MODEL_CATALOG_VERSION);
    snapshot.models = snapshot.models.into_iter().map(sanitize_model).collect();
    snapshot
}

fn parse_catalog(contents: &str) -> Result<ModelCatalogSnapshot, String> {
    let parsed = serde_json::from_str::<DiskCatalogFormat>(contents)
        .map_err(|err| format!("failed to parse model catalog JSON: {err}"))?;
    Ok(match parsed {
        DiskCatalogFormat::Snapshot(snapshot) => sanitize_snapshot(snapshot),
        DiskCatalogFormat::Models(models) => sanitize_snapshot(ModelCatalogSnapshot {
            version: MODEL_CATALOG_VERSION,
            generated_at: None,
            models,
        }),
    })
}

/// Load a catalog snapshot from a specific path.
pub fn load_catalog_from_path(path: &Path) -> Result<ModelCatalogSnapshot, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    parse_catalog(&contents)
}

fn embedded_catalog() -> ModelCatalogSnapshot {
    let fallback = include_str!("../../../registry/models.json");
    parse_catalog(fallback).expect("embedded models_catalog.json must be valid")
}

fn disk_catalog_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    paths.push(
        thinclaw_platform::state_paths()
            .home
            .join("registry/models.json"),
    );
    if let Some(registry_dir) = find_registry_dir() {
        let candidate = registry_dir.join("models.json");
        if !paths.iter().any(|existing| existing == &candidate) {
            paths.push(candidate);
        }
    }
    paths
}

/// Return the first existing disk-backed catalog path.
pub fn disk_catalog_path() -> Option<PathBuf> {
    disk_catalog_paths().into_iter().find(|path| path.is_file())
}

/// Preferred path to write a refreshed catalog.
pub fn preferred_catalog_write_path() -> PathBuf {
    if let Some(existing) = disk_catalog_path() {
        return existing;
    }
    if let Some(registry_dir) = find_registry_dir() {
        return registry_dir.join("models.json");
    }
    thinclaw_platform::state_paths()
        .home
        .join("registry/models.json")
}

fn find_registry_dir() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join("registry");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let mut dir = Some(parent);
        for _ in 0..3 {
            if let Some(d) = dir {
                let candidate = d.join("registry");
                if candidate.is_dir() {
                    return Some(candidate);
                }
                dir = d.parent();
            }
        }
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for candidate in [
        manifest_dir.join("registry"),
        manifest_dir.join("../../registry"),
    ] {
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    None
}

fn load_catalog() -> ModelCatalogSnapshot {
    for path in disk_catalog_paths() {
        if !path.is_file() {
            continue;
        }
        match load_catalog_from_path(&path) {
            Ok(snapshot) if !snapshot.models.is_empty() => {
                tracing::info!(
                    path = %path.display(),
                    models = snapshot.models.len(),
                    "Loaded model compat catalog from disk"
                );
                return snapshot;
            }
            Ok(_) => {
                tracing::warn!(
                    path = %path.display(),
                    "Model compat catalog was empty on disk, using embedded fallback"
                );
            }
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "Failed to load disk model compat catalog, using embedded fallback"
                );
            }
        }
    }

    let embedded = embedded_catalog();
    tracing::info!(
        models = embedded.models.len(),
        "Loaded embedded model compat catalog fallback"
    );
    embedded
}

static MODEL_CATALOG: LazyLock<ModelCatalogSnapshot> = LazyLock::new(load_catalog);

/// Return the current loaded catalog snapshot.
pub fn catalog_snapshot() -> ModelCatalogSnapshot {
    MODEL_CATALOG.clone()
}

/// Build the known model compatibility database.
pub fn known_models() -> Vec<ModelCompat> {
    MODEL_CATALOG.models.clone()
}

/// Persist a catalog snapshot to disk.
pub fn write_catalog_snapshot(path: &Path, snapshot: &ModelCatalogSnapshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(snapshot)
        .map_err(|err| format!("failed to serialize model catalog: {err}"))?;
    std::fs::write(path, json).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

/// Normalize a model lookup key for alias/snapshot matching.
pub fn normalize_lookup_id(model_id: &str) -> String {
    let mut id = model_id.trim();

    if let Some(stripped) = id.strip_prefix('~') {
        id = stripped;
    }

    if let Some((_, tail)) = id.rsplit_once('/') {
        id = tail;
    }

    let mut normalized = id.to_ascii_lowercase();

    // AWS Bedrock model IDs use `anthropic.<model>-v1:0`, and some endpoints
    // may prepend a region segment.
    if let Some(idx) = normalized.find("claude-") {
        normalized = normalized[idx..].to_string();
    }
    if let Some((base, suffix)) = normalized.rsplit_once("-v")
        && suffix
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == ':' || ch == '.')
    {
        normalized = base.to_string();
    }

    // Vertex AI Claude aliases use `claude-...@YYYYMMDD`.
    if let Some((base, snapshot)) = normalized.split_once('@')
        && base.starts_with("claude-")
        && snapshot.chars().all(|ch| ch.is_ascii_digit())
    {
        normalized = format!("{base}-{snapshot}");
    }

    normalized
}

/// Look up a model by ID.
pub fn find_model(model_id: &str) -> Option<ModelCompat> {
    let normalized = normalize_lookup_id(model_id);
    known_models().into_iter().find(|model| {
        model.model_id.eq_ignore_ascii_case(model_id)
            || model.display_name.eq_ignore_ascii_case(model_id)
            || normalize_lookup_id(&model.model_id) == normalized
    })
}

/// List models by provider.
pub fn models_by_provider(provider: &str) -> Vec<ModelCompat> {
    let mut models: Vec<_> = known_models()
        .into_iter()
        .filter(|model| model.provider.eq_ignore_ascii_case(provider))
        .collect();
    models.sort_by(|left, right| {
        left.alias_of
            .is_some()
            .cmp(&right.alias_of.is_some())
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_models_not_empty() {
        assert!(!known_models().is_empty());
    }

    #[test]
    fn test_find_model() {
        let model = find_model("gpt-4o");
        assert!(model.is_some());
        assert_eq!(model.unwrap().context_window, 128_000);
    }

    #[test]
    fn test_find_model_case_insensitive() {
        let model = find_model("GPT-4o");
        assert!(model.is_some());
    }

    #[test]
    fn test_find_model_accepts_provider_prefix() {
        let model = find_model("openai/gpt-5.4");
        assert!(model.is_some());
        assert_eq!(model.unwrap().provider, "openai");
    }

    #[test]
    fn test_find_model_not_found() {
        assert!(find_model("nonexistent-model").is_none());
    }

    #[test]
    fn test_is_frontier() {
        let model = find_model("gpt-4o").unwrap();
        assert!(model.is_frontier());
    }

    #[test]
    fn test_is_budget() {
        let model = find_model("gemini-2.0-flash").unwrap();
        assert!(model.is_budget());
    }

    #[test]
    fn test_estimate_cost() {
        let model = find_model("gpt-4o").unwrap();
        let cost = model.estimate_cost(1000, 500).unwrap();
        assert!(cost > 0.0);
    }

    #[test]
    fn test_models_by_provider() {
        let openai = models_by_provider("openai");
        assert!(!openai.is_empty());
        assert!(openai.iter().all(|m| m.provider == "openai"));
    }

    #[test]
    fn test_routing_quality_score_orders_known_models() {
        let opus = find_model("claude-opus-4-7")
            .unwrap()
            .routing_quality_score();
        let gpt_54 = find_model("gpt-5.4").unwrap().routing_quality_score();
        let gpt_54_mini = find_model("gpt-5.4-mini").unwrap().routing_quality_score();
        let pi_ai = find_model("pi-ai").unwrap().routing_quality_score();

        assert!(gpt_54 > gpt_54_mini);
        assert!(opus > pi_ai);
        assert!(gpt_54_mini > pi_ai);
        assert!((0.0..=1.0).contains(&opus));
        assert!((0.0..=1.0).contains(&gpt_54));
    }

    #[test]
    fn test_estimate_routing_quality_never_returns_unknown_bucket() {
        let score = estimate_routing_quality(None, None, None, None, None, None, None, None, None);
        assert!((0.0..=1.0).contains(&score));
        assert_ne!(
            score, 0.5,
            "fallback quality should not collapse to a generic unknown score"
        );
    }

    #[test]
    fn test_parse_snapshot_preserves_alias_metadata() {
        let parsed = parse_catalog(
            r#"{
                "version": 1,
                "generated_at": "2026-04-23T00:00:00Z",
                "models": [
                    {
                        "provider": "anthropic",
                        "model_id": "claude-sonnet-4-6",
                        "alias_of": "claude-sonnet-4-20250514",
                        "display_name": "Claude Sonnet 4.6",
                        "context_window": 200000,
                        "max_output_tokens": 64000,
                        "supports_tools": true,
                        "supports_vision": true,
                        "supports_streaming": true,
                        "supports_thinking": true,
                        "pricing_input": 3.0,
                        "pricing_output": 15.0,
                        "source_url": "https://docs.anthropic.com/en/docs/about-claude/models/overview",
                        "fetched_at": "2026-04-23T00:00:00Z"
                    }
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(parsed.models.len(), 1);
        assert_eq!(
            parsed.models[0].alias_of.as_deref(),
            Some("claude-sonnet-4-20250514")
        );
    }

    #[test]
    fn test_parse_array_format_is_still_supported() {
        let parsed = parse_catalog(
            r#"[
                {
                    "provider": "openai",
                    "model_id": "gpt-4o",
                    "display_name": "GPT-4o",
                    "context_window": 128000,
                    "max_output_tokens": 16384,
                    "supports_tools": true,
                    "supports_vision": true,
                    "supports_streaming": true,
                    "supports_thinking": false
                }
            ]"#,
        )
        .unwrap();

        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].model_id, "gpt-4o");
    }

    #[test]
    fn test_normalize_lookup_id_strips_provider_and_vertex_snapshot() {
        assert_eq!(
            normalize_lookup_id("openrouter/anthropic/claude-sonnet-4-20250514"),
            "claude-sonnet-4-20250514"
        );
        assert_eq!(
            normalize_lookup_id("claude-sonnet-4-5@20250929"),
            "claude-sonnet-4-5-20250929"
        );
    }
}
