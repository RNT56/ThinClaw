//! Background pricing synchronization via OpenRouter API.
//!
//! Fetches current per-token pricing from OpenRouter's public `/api/v1/models`
//! endpoint and updates the dynamic cost overlay in [`crate::llm::costs`].
//!
//! # Design
//!
//! - **No API key required**: The OpenRouter models endpoint is public.
//! - **Fetch frequency**: Once at startup + every 24 hours.
//! - **Persistence**: Fetched pricing is stored in the database so restarts
//!   don't require immediate network access.
//! - **Fallback chain**: Dynamic overlay → Static table → `default_cost()`.
//! - **Error resilience**: Fetch failures are logged but never block startup
//!   or normal operation.

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::llm::costs;

/// OpenRouter API response shape.
#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    data: Vec<OpenRouterModel>,
}

/// A single model entry from OpenRouter.
#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    pricing: Option<OpenRouterPricing>,
}

/// Per-token pricing from OpenRouter.
///
/// Values are strings representing cost per token in USD
/// (e.g. `"0.00000013"` for $0.13 per 1M tokens).
#[derive(Debug, Deserialize)]
struct OpenRouterPricing {
    prompt: Option<String>,
    completion: Option<String>,
}

/// Cached pricing data that can be persisted to the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingCache {
    /// ISO 8601 timestamp of when the data was fetched.
    pub fetched_at: String,
    /// Map of model_id → (input_cost_per_token, output_cost_per_token) as string pairs.
    pub models: HashMap<String, (String, String)>,
}

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const PRICING_DB_NAMESPACE: &str = "system";
const PRICING_DB_KEY: &str = "pricing_cache";

/// Fetch current pricing from OpenRouter's public API.
///
/// Returns a map of `model_id → (input_cost_per_token, output_cost_per_token)`
/// as `Decimal` pairs. Models with zero or missing pricing are excluded.
pub async fn fetch_openrouter_pricing() -> Result<HashMap<String, (Decimal, Decimal)>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let response = client
        .get(OPENROUTER_MODELS_URL)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenRouter models: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "OpenRouter API returned status {}",
            response.status()
        ));
    }

    let body: OpenRouterResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenRouter response: {}", e))?;

    let mut pricing = HashMap::new();

    for model in body.data {
        let Some(price) = model.pricing else {
            continue;
        };
        let Some(prompt_str) = price.prompt else {
            continue;
        };
        let Some(completion_str) = price.completion else {
            continue;
        };

        // Parse as Decimal for precision
        let Ok(input_cost) = prompt_str.parse::<Decimal>() else {
            continue;
        };
        let Ok(output_cost) = completion_str.parse::<Decimal>() else {
            continue;
        };

        // Skip free/zero-cost models (local models handled by static table)
        if input_cost.is_zero() && output_cost.is_zero() {
            continue;
        }

        pricing.insert(model.id, (input_cost, output_cost));
    }

    tracing::info!(
        models_fetched = pricing.len(),
        "Fetched pricing from OpenRouter"
    );

    Ok(pricing)
}

/// Convert fetched pricing into a serializable cache.
fn to_cache(pricing: &HashMap<String, (Decimal, Decimal)>) -> PricingCache {
    PricingCache {
        fetched_at: chrono::Utc::now().to_rfc3339(),
        models: pricing
            .iter()
            .map(|(id, (input, output))| (id.clone(), (input.to_string(), output.to_string())))
            .collect(),
    }
}

/// Restore pricing from a serialized cache.
fn from_cache(cache: &PricingCache) -> HashMap<String, (Decimal, Decimal)> {
    cache
        .models
        .iter()
        .filter_map(|(id, (input_str, output_str))| {
            let input = input_str.parse::<Decimal>().ok()?;
            let output = output_str.parse::<Decimal>().ok()?;
            Some((id.clone(), (input, output)))
        })
        .collect()
}

/// Attempt to load cached pricing from the database.
pub async fn load_from_db(db: &dyn crate::db::Database) -> Option<HashMap<String, (Decimal, Decimal)>> {
    match db.get_setting(PRICING_DB_NAMESPACE, PRICING_DB_KEY).await {
        Ok(Some(json_value)) => {
            match serde_json::from_value::<PricingCache>(json_value) {
                Ok(cache) => {
                    let pricing = from_cache(&cache);
                    tracing::info!(
                        models = pricing.len(),
                        fetched_at = %cache.fetched_at,
                        "Loaded pricing cache from database"
                    );
                    Some(pricing)
                }
                Err(e) => {
                    tracing::warn!("Failed to parse pricing cache from DB: {}", e);
                    None
                }
            }
        }
        Ok(None) => {
            tracing::debug!("No pricing cache found in database");
            None
        }
        Err(e) => {
            tracing::warn!("Failed to read pricing cache from DB: {}", e);
            None
        }
    }
}

/// Persist pricing to the database for cross-restart durability.
pub async fn save_to_db(
    db: &dyn crate::db::Database,
    pricing: &HashMap<String, (Decimal, Decimal)>,
) {
    let cache = to_cache(pricing);
    match serde_json::to_value(&cache) {
        Ok(json_value) => {
            if let Err(e) = db
                .set_setting(PRICING_DB_NAMESPACE, PRICING_DB_KEY, &json_value)
                .await
            {
                tracing::warn!("Failed to persist pricing cache to DB: {}", e);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to serialize pricing cache: {}", e);
        }
    }
}

/// Run a single pricing sync cycle: fetch → update overlay → persist.
///
/// Returns `true` if the overlay was successfully updated.
pub async fn sync_once(db: Option<&dyn crate::db::Database>) -> bool {
    match fetch_openrouter_pricing().await {
        Ok(pricing) => {
            let count = pricing.len();
            costs::set_dynamic_pricing(pricing.clone());
            tracing::info!(models = count, "Updated dynamic pricing overlay");

            // Persist to DB if available
            if let Some(db) = db {
                save_to_db(db, &pricing).await;
            }

            true
        }
        Err(e) => {
            tracing::warn!("Pricing sync failed: {}", e);
            false
        }
    }
}

/// Spawn the background pricing sync task.
///
/// Runs an initial sync immediately, then repeats every 24 hours.
/// If a database is provided, cached pricing is loaded first (for instant
/// startup) and updated data is persisted after each successful fetch.
pub fn spawn_pricing_sync(
    db: Option<std::sync::Arc<dyn crate::db::Database>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Step 1: Try loading from DB cache for instant startup pricing
        if let Some(ref db) = db {
            if let Some(cached) = load_from_db(db.as_ref()).await {
                costs::set_dynamic_pricing(cached);
            }
        }

        // Step 2: Fetch fresh pricing from OpenRouter
        let db_ref = db.as_deref();
        sync_once(db_ref).await;

        // Step 3: Refresh every 24 hours
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        interval.tick().await; // skip the immediate first tick (already did sync above)

        loop {
            interval.tick().await;
            sync_once(db_ref).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_roundtrip() {
        let mut pricing = HashMap::new();
        pricing.insert(
            "openai/gpt-4o".to_string(),
            (
                Decimal::new(25, 7),  // 0.0000025
                Decimal::new(10, 5),  // 0.00010
            ),
        );

        let cache = to_cache(&pricing);
        let restored = from_cache(&cache);

        assert_eq!(restored.len(), 1);
        let (input, output) = restored.get("openai/gpt-4o").unwrap();
        assert_eq!(*input, Decimal::new(25, 7));
        assert_eq!(*output, Decimal::new(10, 5));
    }

    #[test]
    fn test_cache_skips_invalid() {
        let cache = PricingCache {
            fetched_at: "2026-04-07T00:00:00Z".to_string(),
            models: {
                let mut m = HashMap::new();
                m.insert("good-model".to_string(), ("0.001".to_string(), "0.002".to_string()));
                m.insert("bad-model".to_string(), ("not-a-number".to_string(), "0.002".to_string()));
                m
            },
        };

        let restored = from_cache(&cache);
        assert_eq!(restored.len(), 1);
        assert!(restored.contains_key("good-model"));
    }
}
