//! LLM/runner cost attribution and provider hourly-rate estimation.

use std::collections::BTreeMap;

use crate::types::*;

#[derive(Debug, Clone)]
pub struct LlmCostAttribution {
    pub total_usd: f64,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct RunnerCostBreakdown {
    pub total_usd: f64,
    pub details: serde_json::Value,
    pub provider_metadata_overlay: Option<serde_json::Value>,
}

pub fn summarize_llm_usage(
    records: &[ExperimentModelUsageRecord],
    source: &str,
) -> LlmCostAttribution {
    let mut total_usd = 0.0;
    let mut latency_sum_ms: u64 = 0;
    let mut latency_count: u64 = 0;
    let mut by_role: BTreeMap<String, f64> = BTreeMap::new();
    let mut by_provider: BTreeMap<String, f64> = BTreeMap::new();
    let mut by_model: BTreeMap<String, f64> = BTreeMap::new();
    for record in records {
        let cost = record.cost_usd.unwrap_or(0.0);
        total_usd += cost;
        if let Some(latency_ms) = record.latency_ms {
            latency_sum_ms += latency_ms;
            latency_count += 1;
        }
        let role_key = record
            .logical_role
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        *by_role.entry(role_key).or_insert(0.0) += cost;
        *by_provider.entry(record.provider.clone()).or_insert(0.0) += cost;
        *by_model
            .entry(format!("{}/{}", record.provider, record.model))
            .or_insert(0.0) += cost;
    }
    let avg_latency_ms = if latency_count == 0 {
        None
    } else {
        Some(latency_sum_ms as f64 / latency_count as f64)
    };
    LlmCostAttribution {
        total_usd,
        details: serde_json::json!({
            "source": source,
            "usage_record_count": records.len(),
            "total_usd": total_usd,
            "avg_latency_ms": avg_latency_ms,
            "by_role_usd": by_role,
            "by_provider_usd": by_provider,
            "by_model_usd": by_model,
        }),
    }
}

pub fn runner_cost_breakdown(
    trial: &ExperimentTrial,
    reported_runner_cost_usd: Option<f64>,
) -> RunnerCostBreakdown {
    if let Some(cost) = reported_runner_cost_usd.filter(|value| value.is_finite() && *value >= 0.0)
    {
        return RunnerCostBreakdown {
            total_usd: cost,
            details: serde_json::json!({
                "source": "runner_completion",
                "reported": true,
                "total_usd": cost,
            }),
            provider_metadata_overlay: Some(serde_json::json!({
                "cost_estimate": {
                    "estimated": false,
                    "usd": cost,
                    "source": "runner_completion",
                }
            })),
        };
    }
    if let Some(estimate) = estimated_provider_runtime_cost_usd(trial) {
        return RunnerCostBreakdown {
            total_usd: estimate.total_usd,
            details: serde_json::json!({
                "source": estimate.source,
                "estimated": true,
                "total_usd": estimate.total_usd,
                "hourly_rate_usd": estimate.hourly_rate_usd,
                "native_hourly_rate": estimate.native_hourly_rate,
                "native_currency": estimate.native_currency,
                "normalization": estimate.normalization,
            }),
            provider_metadata_overlay: Some(serde_json::json!({
                "cost_estimate": {
                    "estimated": true,
                    "usd": estimate.total_usd,
                    "hourly_rate_usd": estimate.hourly_rate_usd,
                    "native_hourly_rate": estimate.native_hourly_rate,
                    "native_currency": estimate.native_currency,
                    "normalization": estimate.normalization,
                    "source": estimate.source,
                }
            })),
        };
    }
    RunnerCostBreakdown {
        total_usd: 0.0,
        details: serde_json::json!({
            "source": "none",
            "estimated": false,
            "total_usd": 0.0,
        }),
        provider_metadata_overlay: None,
    }
}

pub fn metadata_string_field(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone)]
pub struct ProviderCostEstimate {
    pub total_usd: f64,
    pub hourly_rate_usd: f64,
    pub source: String,
    pub native_hourly_rate: Option<f64>,
    pub native_currency: Option<String>,
    pub normalization: Option<String>,
}

pub type ProviderHourlyRate = (f64, String, Option<f64>, Option<String>, Option<String>);

pub fn estimated_provider_runtime_cost_usd(
    trial: &ExperimentTrial,
) -> Option<ProviderCostEstimate> {
    let runtime_ms = trial.runtime_ms?;
    if runtime_ms == 0 {
        return Some(ProviderCostEstimate {
            total_usd: 0.0,
            hourly_rate_usd: 0.0,
            source: "runtime_ms".to_string(),
            native_hourly_rate: None,
            native_currency: None,
            normalization: None,
        });
    }
    let (hourly_rate_usd, source, native_hourly_rate, native_currency, normalization) =
        provider_hourly_rate_usd(&trial.provider_job_metadata, trial.runner_backend)?;
    if !hourly_rate_usd.is_finite() || hourly_rate_usd < 0.0 {
        return None;
    }
    Some(ProviderCostEstimate {
        total_usd: hourly_rate_usd * (runtime_ms as f64 / 3_600_000.0),
        hourly_rate_usd,
        source,
        native_hourly_rate,
        native_currency,
        normalization,
    })
}

pub fn provider_hourly_rate_usd(
    metadata: &serde_json::Value,
    backend: ExperimentRunnerBackend,
) -> Option<ProviderHourlyRate> {
    match backend {
        ExperimentRunnerBackend::Runpod => numeric_pointer_candidates(
            metadata,
            &[
                "/pod/adjustedCostPerHr",
                "/pod/costPerHr",
                "/launch_request/costPerHr",
            ],
        )
        .map(|(credits_per_hour, source)| {
            // RunPod prices in account credits, not USD. We surface the rate as an
            // approximate USD figure under the assumption 1 credit ≈ 1 USD. This is
            // an approximation, not a billed amount: the `native_currency` /
            // `normalization` fields below carry the assumption forward so it reaches
            // the operator-facing cost surfaces (see WS-07 / RESEARCH_AND_EXPERIMENTS.md).
            (
                credits_per_hour,
                source,
                Some(credits_per_hour),
                Some("runpod_credits".to_string()),
                Some("assumed_1_credit_equals_1_usd".to_string()),
            )
        }),
        ExperimentRunnerBackend::Vast => numeric_pointer_candidates(
            metadata,
            &[
                "/selected_offer/dph_total",
                "/selected_offer/search/totalHour",
                "/selected_offer/totalHour",
                "/instance/dph_total",
                "/instance/search/totalHour",
            ],
        )
        .map(|(usd_per_hour, source)| {
            (
                usd_per_hour,
                source,
                Some(usd_per_hour),
                Some("usd".to_string()),
                None,
            )
        }),
        ExperimentRunnerBackend::Lambda => numeric_pointer_candidates(
            metadata,
            &[
                "/instance/hourly_cost_usd",
                "/instance/usd_per_hour",
                "/instance/price_usd_per_hour",
                "/launch_request/hourly_cost_usd",
                "/launch_request/usd_per_hour",
                "/launch_request/price_usd_per_hour",
            ],
        )
        .map(|(usd_per_hour, source)| {
            (
                usd_per_hour,
                source,
                Some(usd_per_hour),
                Some("usd".to_string()),
                None,
            )
        })
        .or_else(|| {
            numeric_pointer_candidates(
                metadata,
                &[
                    "/instance/price_cents_per_hour",
                    "/launch_request/price_cents_per_hour",
                ],
            )
            .map(|(cents, source)| {
                (
                    cents / 100.0,
                    format!("{source} (converted_from_cents)"),
                    Some(cents),
                    Some("cents".to_string()),
                    Some("converted_from_cents".to_string()),
                )
            })
        }),
        _ => None,
    }
}

fn numeric_pointer_candidates(
    value: &serde_json::Value,
    pointers: &[&str],
) -> Option<(f64, String)> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(json_value_as_f64)
            .map(|value| (value, pointer.trim_start_matches('/').replace('/', ".")))
    })
}

fn json_value_as_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}
