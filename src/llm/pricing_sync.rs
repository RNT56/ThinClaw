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
use tokio::sync::oneshot;

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
const MAX_PRICING_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PRICING_MODELS: usize = 10_000;
const MAX_MODEL_ID_BYTES: usize = 256;
const MAX_PRICE_STRING_BYTES: usize = 64;

fn valid_model_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_MODEL_ID_BYTES && !id.chars().any(char::is_control)
}

fn parse_price_pair(id: &str, input: &str, output: &str) -> Option<(Decimal, Decimal)> {
    if !valid_model_id(id)
        || input.len() > MAX_PRICE_STRING_BYTES
        || output.len() > MAX_PRICE_STRING_BYTES
    {
        return None;
    }
    let input = input.parse::<Decimal>().ok()?;
    let output = output.parse::<Decimal>().ok()?;
    let maximum = Decimal::from(100_u32);
    if input.is_sign_negative()
        || output.is_sign_negative()
        || input > maximum
        || output > maximum
        || (input.is_zero() && output.is_zero())
    {
        return None;
    }
    Some((input, output))
}

/// Fetch current pricing from OpenRouter's public API.
///
/// Returns a map of `model_id → (input_cost_per_token, output_cost_per_token)`
/// as `Decimal` pairs. Models with zero or missing pricing are excluded.
pub async fn fetch_openrouter_pricing() -> Result<HashMap<String, (Decimal, Decimal)>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .user_agent(concat!("thinclaw/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let response = client
        .get(OPENROUTER_MODELS_URL)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenRouter models: {}", e.without_url()))?;

    if !response.status().is_success() {
        return Err(format!(
            "OpenRouter API returned status {}",
            response.status()
        ));
    }

    let body: OpenRouterResponse =
        crate::http_response::bounded_json(response, MAX_PRICING_RESPONSE_BYTES)
            .await
            .map_err(|e| format!("Failed to parse OpenRouter response: {}", e))?;

    let mut pricing = HashMap::new();

    for model in body.data.into_iter().take(MAX_PRICING_MODELS) {
        let Some(price) = model.pricing else {
            continue;
        };
        let Some(prompt_str) = price.prompt else {
            continue;
        };
        let Some(completion_str) = price.completion else {
            continue;
        };

        let Some((input_cost, output_cost)) =
            parse_price_pair(&model.id, &prompt_str, &completion_str)
        else {
            continue;
        };

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
            .filter_map(|(id, (input, output))| {
                let input = input.to_string();
                let output = output.to_string();
                parse_price_pair(id, &input, &output)?;
                Some((id.clone(), (input, output)))
            })
            .take(MAX_PRICING_MODELS)
            .collect(),
    }
}

/// Restore pricing from a serialized cache.
fn from_cache(cache: &PricingCache) -> HashMap<String, (Decimal, Decimal)> {
    cache
        .models
        .iter()
        .take(MAX_PRICING_MODELS)
        .filter_map(|(id, (input_str, output_str))| {
            let (input, output) = parse_price_pair(id, input_str, output_str)?;
            Some((id.clone(), (input, output)))
        })
        .collect()
}

/// Attempt to load cached pricing from the database.
pub async fn load_cache_from_db(db: &dyn crate::db::Database) -> Option<PricingCache> {
    match db.get_setting(PRICING_DB_NAMESPACE, PRICING_DB_KEY).await {
        Ok(Some(json_value)) => match serde_json::from_value::<PricingCache>(json_value) {
            Ok(mut cache) => {
                if cache.fetched_at.len() > 128
                    || chrono::DateTime::parse_from_rfc3339(&cache.fetched_at).is_err()
                {
                    tracing::warn!("Pricing cache has an invalid fetch timestamp");
                    return None;
                }
                cache.models = from_cache(&cache)
                    .into_iter()
                    .map(|(id, (input, output))| (id, (input.to_string(), output.to_string())))
                    .collect();
                tracing::info!(
                    models = cache.models.len(),
                    "Loaded pricing cache from database"
                );
                Some(cache)
            }
            Err(e) => {
                tracing::warn!("Failed to parse pricing cache from DB: {}", e);
                None
            }
        },
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

/// Attempt to load cached pricing from the database.
pub async fn load_from_db(
    db: &dyn crate::db::Database,
) -> Option<HashMap<String, (Decimal, Decimal)>> {
    load_cache_from_db(db).await.map(|cache| from_cache(&cache))
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
        // Keep the sender alive inside the owned task. Dropping it before
        // spawning makes a oneshot receiver immediately readable and used to
        // terminate desktop pricing sync before its first cache load.
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let _shutdown_tx = shutdown_tx;
        run_pricing_sync(db, shutdown_rx).await;
    })
}

/// Spawn the background pricing sync task with cooperative shutdown.
///
/// The task exits before the next long sleep when `shutdown_rx` resolves. If a
/// fetch is in flight, shutdown races that fetch so app teardown does not have
/// to wait on OpenRouter network I/O.
pub fn spawn_pricing_sync_with_shutdown(
    db: Option<std::sync::Arc<dyn crate::db::Database>>,
    shutdown_rx: oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_pricing_sync(db, shutdown_rx))
}

async fn run_pricing_sync(
    db: Option<std::sync::Arc<dyn crate::db::Database>>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    // Step 1: Try loading from DB cache for instant startup pricing
    if let Some(ref db) = db
        && let Some(cache) = tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::info!("Pricing sync stopped before loading DB cache");
                return;
            }
            cache = load_cache_from_db(db.as_ref()) => cache,
        }
    {
        let fetched_at = chrono::DateTime::parse_from_rfc3339(&cache.fetched_at)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc));
        let cached = from_cache(&cache);
        costs::set_dynamic_pricing_with_fetched_at(cached, fetched_at);
    }

    // Step 2: Fetch fresh pricing from OpenRouter
    let db_ref = db.as_deref();
    tokio::select! {
        _ = &mut shutdown_rx => {
            tracing::info!("Pricing sync stopped before initial fetch");
            return;
        }
        _ = sync_once(db_ref) => {}
    }

    // Step 3: Refresh every 24 hours
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
    interval.tick().await; // skip the immediate first tick (already did sync above)

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::info!("Pricing sync stopped");
                break;
            }
            _ = interval.tick() => {}
        }
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::info!("Pricing sync stopped before scheduled fetch");
                break;
            }
            _ = sync_once(db_ref) => {}
        }
    }
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
                Decimal::new(25, 7), // 0.0000025
                Decimal::new(10, 5), // 0.00010
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
                m.insert(
                    "good-model".to_string(),
                    ("0.001".to_string(), "0.002".to_string()),
                );
                m.insert(
                    "bad-model".to_string(),
                    ("not-a-number".to_string(), "0.002".to_string()),
                );
                m
            },
        };

        let restored = from_cache(&cache);
        assert_eq!(restored.len(), 1);
        assert!(restored.contains_key("good-model"));
    }

    #[test]
    fn pricing_policy_rejects_negative_extreme_and_oversized_values() {
        assert!(parse_price_pair("model", "0.001", "0.002").is_some());
        assert!(parse_price_pair("model", "-0.001", "0.002").is_none());
        assert!(parse_price_pair("model", "101", "0.002").is_none());
        assert!(parse_price_pair("model", "0", "0").is_none());
        assert!(parse_price_pair(&"x".repeat(MAX_MODEL_ID_BYTES + 1), "1", "1").is_none());
        assert!(parse_price_pair("model", &"1".repeat(MAX_PRICE_STRING_BYTES + 1), "1").is_none());
    }

    #[test]
    fn cache_policy_drops_poisoned_prices() {
        let cache = PricingCache {
            fetched_at: "2026-04-07T00:00:00Z".to_string(),
            models: HashMap::from([
                ("negative".to_string(), ("-1".to_string(), "1".to_string())),
                ("free".to_string(), ("0".to_string(), "0".to_string())),
                ("valid".to_string(), ("0.1".to_string(), "0.2".to_string())),
            ]),
        };
        let restored = from_cache(&cache);
        assert_eq!(restored.len(), 1);
        assert!(restored.contains_key("valid"));
    }

    #[tokio::test]
    async fn test_pricing_sync_with_shutdown_exits_before_fetch() {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        shutdown_tx
            .send(())
            .expect("pricing sync receiver should still exist");

        let handle = spawn_pricing_sync_with_shutdown(None, shutdown_rx);
        tokio::time::timeout(std::time::Duration::from_millis(250), handle)
            .await
            .expect("pricing sync should stop promptly")
            .expect("pricing sync task should not panic");
    }

    #[tokio::test]
    async fn no_signal_pricing_sync_does_not_self_cancel() {
        let handle = spawn_pricing_sync(None);
        tokio::task::yield_now().await;
        assert!(
            !handle.is_finished(),
            "no-signal pricing sync must remain alive for scheduled refreshes"
        );
        handle.abort();
        let _ = handle.await;
    }
}
