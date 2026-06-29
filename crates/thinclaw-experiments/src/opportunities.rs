//! Opportunity derivation, scoring, summaries, and usage classification.

use std::collections::{HashMap, HashSet};

use crate::types::*;
use chrono::{DateTime, Utc};
use thinclaw_history::OutcomeContract;
use uuid::Uuid;

pub fn target_signature(
    kind: ExperimentTargetKind,
    metadata: &serde_json::Value,
) -> Option<String> {
    let provider = metadata
        .get("provider")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let model = metadata
        .get("model")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let route_key = metadata
        .get("route_key")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let asset_id = metadata
        .get("asset_id")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();

    let mut parts = vec![format!("{kind:?}")];
    if !provider.is_empty() {
        parts.push(provider);
    }
    if !model.is_empty() {
        parts.push(model);
    }
    if !route_key.is_empty() {
        parts.push(route_key);
    }
    if !asset_id.is_empty() {
        parts.push(asset_id);
    }
    if parts.len() == 1 {
        return None;
    }
    Some(parts.join("|"))
}

pub fn ensure_unique_target_signature(
    kind: ExperimentTargetKind,
    metadata: &serde_json::Value,
    skip_target_id: Option<Uuid>,
    targets: &[ExperimentTarget],
) -> Result<(), String> {
    let Some(signature) = target_signature(kind, metadata) else {
        return Ok(());
    };
    if targets.iter().any(|existing| {
        existing.kind == kind
            && skip_target_id != Some(existing.id)
            && target_signature(existing.kind, &existing.metadata).as_deref()
                == Some(signature.as_str())
    }) {
        return Err(format!(
            "Duplicate target for linked identity '{signature}'"
        ));
    }
    Ok(())
}

pub fn derive_opportunities(
    usage: &[ExperimentModelUsageRecord],
    targets: &[ExperimentTarget],
    target_links: &[ExperimentTargetLink],
) -> Vec<ExperimentOpportunity> {
    let mut opportunities_by_key: HashMap<String, OpportunityAggregate> = HashMap::new();

    for record in usage {
        let class = usage_classification(record);
        let route_key = record.route_key.clone();
        let logical_role = record.logical_role.clone();
        let candidate_kinds = candidate_kinds_for_usage(record, class, targets);

        for kind in candidate_kinds {
            let key = opportunity_key_string(
                &record.provider,
                &record.model,
                route_key.as_deref(),
                logical_role.as_deref(),
                kind,
            );
            let linked_target_id = find_linked_target_id(target_links, targets, record, kind)
                .or_else(|| find_linked_target(targets, record, kind).map(|target| target.id));
            let aggregate =
                opportunities_by_key
                    .entry(key)
                    .or_insert_with(|| OpportunityAggregate {
                        provider: record.provider.clone(),
                        model: record.model.clone(),
                        route_key: route_key.clone(),
                        logical_role: logical_role.clone(),
                        kind,
                        class,
                        call_count: 0,
                        error_count: 0,
                        latency_sum_ms: 0,
                        cost_sum_usd: 0.0,
                        first_seen: record.created_at,
                        last_seen: record.created_at,
                        linked_target_id,
                    });
            aggregate.call_count = aggregate.call_count.saturating_add(1);
            if !record.success {
                aggregate.error_count = aggregate.error_count.saturating_add(1);
            }
            aggregate.last_seen = aggregate.last_seen.max(record.created_at);
            aggregate.first_seen = aggregate.first_seen.min(record.created_at);
            if let Some(linked_target_id) = linked_target_id {
                aggregate.linked_target_id = Some(linked_target_id);
            }
            aggregate.latency_sum_ms = aggregate
                .latency_sum_ms
                .saturating_add(record.latency_ms.unwrap_or_default());
            aggregate.cost_sum_usd += record.cost_usd.unwrap_or(0.0);
        }
    }

    let mut aggregates: Vec<_> = opportunities_by_key.into_values().collect();
    aggregates.sort_by(|left, right| {
        aggregate_opportunity_score(right)
            .partial_cmp(&aggregate_opportunity_score(left))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.call_count.cmp(&right.call_count).reverse())
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.model.cmp(&right.model))
    });

    let mut opportunities = Vec::with_capacity(aggregates.len());
    for aggregate in aggregates {
        let self_hosted = aggregate.class != UsageClass::Hosted;
        let avg_latency_ms = if aggregate.call_count == 0 {
            None
        } else {
            Some(aggregate.latency_sum_ms as f64 / aggregate.call_count as f64)
        };
        let avg_cost_usd = if aggregate.call_count == 0 {
            None
        } else {
            Some(aggregate.cost_sum_usd / aggregate.call_count as f64)
        };
        let error_rate = if aggregate.call_count == 0 {
            0.0
        } else {
            aggregate.error_count as f64 / aggregate.call_count as f64
        };
        let key = opportunity_key_string(
            &aggregate.provider,
            &aggregate.model,
            aggregate.route_key.as_deref(),
            aggregate.logical_role.as_deref(),
            aggregate.kind,
        );
        let hash = blake3::hash(key.as_bytes()).to_hex().to_string();
        let rank_score = aggregate_opportunity_score(&aggregate);
        let route_key = aggregate.route_key.clone();
        let logical_role = aggregate.logical_role.clone();
        let signals =
            opportunity_signals_for_usage(&aggregate, error_rate, avg_latency_ms, avg_cost_usd);
        let summary = opportunity_summary(
            aggregate.kind,
            aggregate.provider.as_str(),
            aggregate.model.as_str(),
            route_key.as_deref(),
            logical_role.as_deref(),
            self_hosted,
        );
        opportunities.push(ExperimentOpportunity {
            id: format!("opp_{}", &hash[..16]),
            provider: aggregate.provider,
            model: aggregate.model,
            route_key: route_key.clone(),
            logical_role,
            opportunity_type: aggregate.kind,
            summary,
            gpu_requirement: opportunity_gpu_requirement(aggregate.kind, self_hosted),
            suggested_preset: opportunity_preset(aggregate.kind, self_hosted),
            linked_target_id: aggregate.linked_target_id,
            source: Some("telemetry".to_string()),
            confidence: Some((0.4 + (aggregate.call_count.min(8) as f64 * 0.05)).clamp(0.4, 0.9)),
            signals,
            project_hint: None,
            metadata: serde_json::json!({
                "usage_class": format!("{:?}", aggregate.class),
                "call_count": aggregate.call_count,
                "error_count": aggregate.error_count,
                "error_rate": error_rate,
                "avg_latency_ms": avg_latency_ms,
                "avg_cost_usd": avg_cost_usd,
                "rank_score": rank_score,
                "linked_target": aggregate.linked_target_id.is_some(),
                "route_key": route_key,
            }),
            created_at: aggregate.first_seen,
            updated_at: aggregate.last_seen,
        });
    }

    opportunities
}

pub fn sort_experiment_opportunities(opportunities: &mut [ExperimentOpportunity]) {
    opportunities.sort_by(|left, right| {
        let right_score = right
            .metadata
            .get("rank_score")
            .and_then(|value| value.as_f64())
            .unwrap_or_default();
        let left_score = left
            .metadata
            .get("rank_score")
            .and_then(|value| value.as_f64())
            .unwrap_or_default();
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub fn experiment_target_kind_sort_key(kind: ExperimentTargetKind) -> &'static str {
    match kind {
        ExperimentTargetKind::PromptAsset => "prompt_asset",
        ExperimentTargetKind::RoutingPolicy => "routing_policy",
        ExperimentTargetKind::RagConfig => "rag_config",
        ExperimentTargetKind::ToolPolicy => "tool_policy",
        ExperimentTargetKind::Evaluator => "evaluator",
        ExperimentTargetKind::Parser => "parser",
        ExperimentTargetKind::InferenceConfig => "inference_config",
        ExperimentTargetKind::TrainingConfig => "training_config",
        ExperimentTargetKind::TrainingCode => "training_code",
        ExperimentTargetKind::ServingConfig => "serving_config",
    }
}

#[derive(Clone)]
struct OpportunityAggregate {
    provider: String,
    model: String,
    route_key: Option<String>,
    logical_role: Option<String>,
    kind: ExperimentTargetKind,
    class: UsageClass,
    call_count: u32,
    error_count: u32,
    latency_sum_ms: u64,
    cost_sum_usd: f64,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    linked_target_id: Option<Uuid>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UsageClass {
    Hosted,
    SelfHosted,
    CustomHostedOrSelf,
}

impl std::fmt::Debug for UsageClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hosted => f.write_str("hosted"),
            Self::SelfHosted => f.write_str("self_hosted"),
            Self::CustomHostedOrSelf => f.write_str("custom_hosted_or_self_hosted"),
        }
    }
}

fn usage_classification(record: &ExperimentModelUsageRecord) -> UsageClass {
    let provider = record.provider.to_ascii_lowercase();
    let endpoint_type = record
        .endpoint_type
        .clone()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let base_url = metadata_string(&record.metadata, "base_url")
        .unwrap_or_default()
        .to_ascii_lowercase();

    if is_known_hosted_provider(&provider) {
        return UsageClass::Hosted;
    }

    if is_known_self_hosted_provider(&provider)
        || endpoint_type.contains("local")
        || endpoint_type.contains("self")
        || endpoint_type.contains("cluster")
        || endpoint_type.contains("private")
        || base_url.contains("localhost")
        || base_url.contains("127.0.0.1")
        || base_url.contains("0.0.0.0")
    {
        return UsageClass::SelfHosted;
    }

    if endpoint_type.contains("openai-compatible")
        || metadata_bool(&record.metadata, "openai_compatible")
        || metadata_bool(&record.metadata, "openai_compatible_or_self_hosted")
    {
        return UsageClass::CustomHostedOrSelf;
    }

    UsageClass::CustomHostedOrSelf
}

fn is_known_hosted_provider(provider: &str) -> bool {
    const KNOWN_HOSTED: &[&str] = &[
        "openai",
        "anthropic",
        "gemini",
        "google",
        "cohere",
        "mistral",
        "azure",
        "perplexity",
        "xai",
        "deepseek",
        "groq",
    ];
    let provider = provider.to_ascii_lowercase();
    KNOWN_HOSTED
        .iter()
        .any(|name| provider == *name || provider.contains(name))
}

fn is_known_self_hosted_provider(provider: &str) -> bool {
    const SELF_HOSTED: &[&str] = &[
        "ollama",
        "lmstudio",
        "vllm",
        "llama_cpp",
        "llama-cpp",
        "llamacpp",
        "localai",
        "tgi",
    ];
    let provider = provider.to_ascii_lowercase();
    SELF_HOSTED
        .iter()
        .any(|name| provider == *name || provider.contains(name))
}

fn metadata_string(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
}

fn metadata_bool(metadata: &serde_json::Value, key: &str) -> bool {
    metadata
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or_default()
}

fn candidate_kinds_for_usage(
    record: &ExperimentModelUsageRecord,
    class: UsageClass,
    targets: &[ExperimentTarget],
) -> Vec<ExperimentTargetKind> {
    let mut kinds = Vec::new();

    if record.route_key.is_some() || record.logical_role.is_some() {
        kinds.push(ExperimentTargetKind::RoutingPolicy);
    }
    if !record.prompt_asset_ids.is_empty() || record.route_key.is_some() {
        kinds.push(ExperimentTargetKind::PromptAsset);
    }
    if !record.retrieval_asset_ids.is_empty() {
        kinds.push(ExperimentTargetKind::RagConfig);
    }
    if !record.tool_policy_ids.is_empty() {
        kinds.push(ExperimentTargetKind::ToolPolicy);
    }
    if !record.success
        || record
            .workload_tag
            .as_deref()
            .map(|value| {
                let value = value.to_ascii_lowercase();
                value.contains("parse")
                    || value.contains("json")
                    || value.contains("structured")
                    || value.contains("extract")
            })
            .unwrap_or(false)
    {
        kinds.push(ExperimentTargetKind::Parser);
    }
    if !record.parser_ids.is_empty() {
        kinds.push(ExperimentTargetKind::Parser);
    }
    if record
        .workload_tag
        .as_deref()
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("eval") || value.contains("judge") || value.contains("score")
        })
        .unwrap_or(false)
    {
        kinds.push(ExperimentTargetKind::Evaluator);
    }
    if !record.evaluator_ids.is_empty() {
        kinds.push(ExperimentTargetKind::Evaluator);
    }

    match class {
        UsageClass::Hosted => {}
        UsageClass::SelfHosted => {
            kinds.extend([
                ExperimentTargetKind::InferenceConfig,
                ExperimentTargetKind::ServingConfig,
                ExperimentTargetKind::TrainingConfig,
                ExperimentTargetKind::TrainingCode,
            ]);
        }
        UsageClass::CustomHostedOrSelf => {
            kinds.extend(
                [
                    ExperimentTargetKind::InferenceConfig,
                    ExperimentTargetKind::ServingConfig,
                    ExperimentTargetKind::TrainingConfig,
                    ExperimentTargetKind::TrainingCode,
                ]
                .into_iter()
                .filter(|kind| find_linked_target(targets, record, *kind).is_some()),
            );
        }
    }

    if kinds.is_empty() {
        kinds.push(ExperimentTargetKind::PromptAsset);
    }

    let mut seen = HashSet::new();
    kinds.retain(|kind| seen.insert(*kind as u8));
    kinds.sort_by_key(|kind| *kind as u8);
    kinds
}

fn opportunity_key_string(
    provider: &str,
    model: &str,
    route_key: Option<&str>,
    logical_role: Option<&str>,
    kind: ExperimentTargetKind,
) -> String {
    format!(
        "{provider}|{model}|{}|{}|{:?}",
        route_key.unwrap_or(""),
        logical_role.unwrap_or(""),
        kind,
    )
}

fn aggregate_opportunity_score(aggregate: &OpportunityAggregate) -> f64 {
    if aggregate.call_count == 0 {
        return 0.0;
    }
    let error_rate = aggregate.error_count as f64 / aggregate.call_count as f64;
    let avg_latency = aggregate.latency_sum_ms as f64 / aggregate.call_count as f64;
    let avg_cost = aggregate.cost_sum_usd / aggregate.call_count as f64;
    let missing_link_penalty = if aggregate.linked_target_id.is_none()
        && matches!(
            aggregate.kind,
            ExperimentTargetKind::InferenceConfig
                | ExperimentTargetKind::ServingConfig
                | ExperimentTargetKind::TrainingConfig
                | ExperimentTargetKind::TrainingCode,
        ) {
        1.25
    } else {
        0.0
    };
    let gpu_penalty = if matches!(aggregate.kind, ExperimentTargetKind::TrainingCode) {
        -2.0
    } else if matches!(
        aggregate.kind,
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig
    ) {
        -1.0
    } else {
        0.0
    };
    aggregate.call_count as f64 * 2.0
        - (error_rate * 100.0)
        - (avg_latency.min(4000.0) / 60.0)
        - avg_cost
        + gpu_penalty
        - missing_link_penalty
}

fn find_linked_target<'a>(
    targets: &'a [ExperimentTarget],
    record: &ExperimentModelUsageRecord,
    kind: ExperimentTargetKind,
) -> Option<&'a ExperimentTarget> {
    targets.iter().find(|target| {
        if target.kind != kind {
            return false;
        }
        let provider_match = target
            .metadata
            .get("provider")
            .and_then(|value| value.as_str())
            .map(|value| value.eq_ignore_ascii_case(&record.provider))
            .unwrap_or(false);
        let model_match = target
            .metadata
            .get("model")
            .and_then(|value| value.as_str())
            .map(|value| value.eq_ignore_ascii_case(&record.model))
            .unwrap_or(false);
        let route_match = target
            .metadata
            .get("route_key")
            .and_then(|value| value.as_str())
            .zip(record.route_key.as_deref())
            .map(|(left, right)| left == right)
            .unwrap_or(false);
        let asset_id_match = target
            .metadata
            .get("asset_id")
            .and_then(|value| value.as_str())
            .map(|asset_id| {
                record.prompt_asset_ids.iter().any(|id| id == asset_id)
                    || record.retrieval_asset_ids.iter().any(|id| id == asset_id)
                    || record.tool_policy_ids.iter().any(|id| id == asset_id)
            })
            .unwrap_or(false);
        provider_match || model_match || route_match || asset_id_match
    })
}

fn find_linked_target_id(
    target_links: &[ExperimentTargetLink],
    targets: &[ExperimentTarget],
    record: &ExperimentModelUsageRecord,
    kind: ExperimentTargetKind,
) -> Option<Uuid> {
    let route_key = record.route_key.as_deref().unwrap_or_default();
    let logical_role = record.logical_role.as_deref().unwrap_or_default();

    target_links
        .iter()
        .find(|link| {
            link.kind == kind
                && link.provider.eq_ignore_ascii_case(&record.provider)
                && link.model.eq_ignore_ascii_case(&record.model)
                && link.route_key.as_deref().unwrap_or_default() == route_key
                && link.logical_role.as_deref().unwrap_or_default() == logical_role
                && targets
                    .iter()
                    .any(|target| target.id == link.target_id && target.kind == kind)
        })
        .map(|link| link.target_id)
}

fn opportunity_summary(
    kind: ExperimentTargetKind,
    provider: &str,
    model: &str,
    route_key: Option<&str>,
    logical_role: Option<&str>,
    self_hosted: bool,
) -> String {
    match kind {
        ExperimentTargetKind::PromptAsset => format!(
            "Optimize prompts and system instructions for {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::RoutingPolicy => format!(
            "Tune routing and fallback policy for {} on {} (route: {}, role: {}).",
            model,
            provider,
            route_key.unwrap_or("default route"),
            logical_role.unwrap_or("default role")
        ),
        ExperimentTargetKind::RagConfig => format!(
            "Improve retrieval and ranking for {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::ToolPolicy => format!(
            "Refine tool selection and execution policy around {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::InferenceConfig => format!(
            "Tune inference parameters for self-hosted model {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::ServingConfig => format!(
            "Adjust serving/runtime settings for self-hosted model {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::TrainingConfig => format!(
            "Benchmark fine-tuning or training configuration for {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::TrainingCode => format!(
            "Improve training code or benchmark harness for {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::Evaluator | ExperimentTargetKind::Parser => {
            if self_hosted {
                format!(
                    "Improve evaluator and parsing reliability around {} on {}.",
                    model, provider
                )
            } else {
                format!(
                    "Tighten evaluator and output parsing around {} on {}.",
                    model, provider
                )
            }
        }
    }
}

fn opportunity_signals_for_usage(
    aggregate: &OpportunityAggregate,
    error_rate: f64,
    avg_latency_ms: Option<f64>,
    avg_cost_usd: Option<f64>,
) -> Vec<String> {
    let mut signals = vec![format!(
        "{} model call{}",
        aggregate.call_count,
        if aggregate.call_count == 1 { "" } else { "s" }
    )];
    if error_rate > 0.0 {
        signals.push(format!("{:.0}% error rate", error_rate * 100.0));
    }
    if let Some(avg_latency_ms) = avg_latency_ms {
        signals.push(format!("{:.0} ms avg latency", avg_latency_ms));
    }
    if let Some(avg_cost_usd) = avg_cost_usd {
        signals.push(format!("${:.4} avg cost", avg_cost_usd));
    }
    signals
}

fn opportunity_gpu_requirement(
    kind: ExperimentTargetKind,
    self_hosted: bool,
) -> ExperimentGpuRequirement {
    if !self_hosted {
        return ExperimentGpuRequirement::NotNeeded;
    }
    match kind {
        ExperimentTargetKind::TrainingConfig | ExperimentTargetKind::TrainingCode => {
            ExperimentGpuRequirement::Required
        }
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig => {
            ExperimentGpuRequirement::Recommended
        }
        _ => ExperimentGpuRequirement::NotNeeded,
    }
}

fn opportunity_preset(kind: ExperimentTargetKind, self_hosted: bool) -> ExperimentPreset {
    match kind {
        ExperimentTargetKind::PromptAsset | ExperimentTargetKind::RoutingPolicy => {
            ExperimentPreset::HostedPromptRouting
        }
        ExperimentTargetKind::RagConfig => ExperimentPreset::RagPipeline,
        ExperimentTargetKind::ToolPolicy
        | ExperimentTargetKind::Evaluator
        | ExperimentTargetKind::Parser => ExperimentPreset::ToolOrchestration,
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig => {
            ExperimentPreset::OpenWeightsInferenceTuning
        }
        ExperimentTargetKind::TrainingConfig => ExperimentPreset::SelfHostedFinetune,
        ExperimentTargetKind::TrainingCode => {
            if self_hosted {
                ExperimentPreset::OpenWeightsTrainingCode
            } else {
                ExperimentPreset::AutoresearchSingleFile
            }
        }
    }
}

pub fn derive_outcome_opportunities(
    contracts: &[OutcomeContract],
    targets: &[ExperimentTarget],
    limit: usize,
    default_prompt_asset: &str,
) -> Vec<ExperimentOpportunity> {
    let cutoff = Utc::now() - chrono::Duration::days(30);
    let mut aggregates: HashMap<String, OutcomeOpportunityAggregate> = HashMap::new();

    for contract in contracts.iter().filter(|contract| {
        contract.final_verdict.as_deref() == Some("negative")
            && contract.evaluated_at.unwrap_or(contract.updated_at) >= cutoff
    }) {
        let Some(kind) = outcome_target_kind(contract) else {
            continue;
        };
        let pattern_key = contract
            .metadata
            .get("pattern_key")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{}:{}:{}",
                    contract.contract_type, contract.source_kind, contract.source_id
                )
            });
        let artifact_type = contract
            .metadata
            .get("artifact_type")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let artifact_name = contract
            .metadata
            .get("artifact_name")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| outcome_default_artifact_name(contract, default_prompt_asset));
        let routine_id = contract
            .metadata
            .get("routine_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let routine_name = contract
            .metadata
            .get("routine_name")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let linked_target_id = find_outcome_linked_target_id(
            targets,
            kind,
            artifact_name.as_deref(),
            routine_id.as_deref(),
            &pattern_key,
        );
        let evaluated_at = contract.evaluated_at.unwrap_or(contract.updated_at);
        let aggregate =
            aggregates
                .entry(pattern_key.clone())
                .or_insert_with(|| OutcomeOpportunityAggregate {
                    kind,
                    contract_type: contract.contract_type.clone(),
                    artifact_type: artifact_type.clone(),
                    artifact_name: artifact_name.clone(),
                    routine_id: routine_id.clone(),
                    routine_name: routine_name.clone(),
                    pattern_key: pattern_key.clone(),
                    count: 0,
                    first_seen: evaluated_at,
                    last_seen: evaluated_at,
                    rank_score: 0.0,
                    linked_target_id,
                });
        aggregate.count = aggregate.count.saturating_add(1);
        aggregate.first_seen = aggregate.first_seen.min(evaluated_at);
        aggregate.last_seen = aggregate.last_seen.max(evaluated_at);
        if aggregate.linked_target_id.is_none() {
            aggregate.linked_target_id = linked_target_id;
        }
    }

    let mut aggregates: Vec<_> = aggregates.into_values().collect();
    for aggregate in &mut aggregates {
        let recency_bonus = ((Utc::now() - aggregate.last_seen).num_days().max(0) as f64).min(14.0);
        aggregate.rank_score = aggregate.count as f64 * 4.0 - recency_bonus;
    }
    aggregates.sort_by(|left, right| {
        right
            .rank_score
            .partial_cmp(&left.rank_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                experiment_target_kind_sort_key(left.kind)
                    .cmp(experiment_target_kind_sort_key(right.kind))
            })
            .then_with(|| left.pattern_key.cmp(&right.pattern_key))
    });

    aggregates
        .into_iter()
        .take(limit.max(1))
        .map(|aggregate| outcome_aggregate_to_opportunity(aggregate, default_prompt_asset))
        .collect()
}

#[derive(Debug, Clone)]
struct OutcomeOpportunityAggregate {
    kind: ExperimentTargetKind,
    contract_type: String,
    artifact_type: Option<String>,
    artifact_name: Option<String>,
    routine_id: Option<String>,
    routine_name: Option<String>,
    pattern_key: String,
    count: u32,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    rank_score: f64,
    linked_target_id: Option<Uuid>,
}

fn outcome_target_kind(contract: &OutcomeContract) -> Option<ExperimentTargetKind> {
    match contract.contract_type.as_str() {
        "turn_usefulness" => Some(ExperimentTargetKind::PromptAsset),
        "routine_usefulness" => Some(ExperimentTargetKind::ToolPolicy),
        "tool_durability" => match contract
            .metadata
            .get("artifact_type")
            .and_then(|value| value.as_str())
        {
            Some("prompt") => Some(ExperimentTargetKind::PromptAsset),
            Some("skill") | Some("routine") => Some(ExperimentTargetKind::ToolPolicy),
            Some("parser") => Some(ExperimentTargetKind::Parser),
            Some("evaluator") => Some(ExperimentTargetKind::Evaluator),
            Some("inference") => Some(ExperimentTargetKind::InferenceConfig),
            Some("serving") => Some(ExperimentTargetKind::ServingConfig),
            Some("training") => Some(ExperimentTargetKind::TrainingConfig),
            Some("training_code") | Some("code") => Some(ExperimentTargetKind::TrainingCode),
            _ if contract.source_kind == "learning_code_proposal" => {
                Some(ExperimentTargetKind::TrainingCode)
            }
            _ => None,
        },
        _ => None,
    }
}

fn outcome_default_artifact_name(
    contract: &OutcomeContract,
    default_prompt_asset: &str,
) -> Option<String> {
    match contract.contract_type.as_str() {
        "turn_usefulness" => Some(default_prompt_asset.to_string()),
        _ => None,
    }
}

fn find_outcome_linked_target_id(
    targets: &[ExperimentTarget],
    kind: ExperimentTargetKind,
    artifact_name: Option<&str>,
    routine_id: Option<&str>,
    pattern_key: &str,
) -> Option<Uuid> {
    targets
        .iter()
        .find(|target| {
            if target.kind != kind {
                return false;
            }
            let asset_id = target
                .metadata
                .get("asset_id")
                .and_then(|value| value.as_str());
            let target_pattern = target
                .metadata
                .get("pattern_key")
                .and_then(|value| value.as_str());
            asset_id
                .zip(artifact_name)
                .is_some_and(|(left, right)| left == right)
                || asset_id
                    .zip(routine_id)
                    .is_some_and(|(left, right)| left == right)
                || target_pattern.is_some_and(|value| value == pattern_key)
        })
        .map(|target| target.id)
}

fn outcome_aggregate_to_opportunity(
    aggregate: OutcomeOpportunityAggregate,
    default_prompt_asset: &str,
) -> ExperimentOpportunity {
    let (summary, project_hint) =
        outcome_summary_and_project_hint(&aggregate, default_prompt_asset);
    let id_source = format!("{}|{:?}", aggregate.pattern_key, aggregate.kind);
    let hash = blake3::hash(id_source.as_bytes()).to_hex().to_string();
    let signals = outcome_signals(&aggregate);
    ExperimentOpportunity {
        id: format!("opp_outcome_{}", &hash[..16]),
        provider: "outcome_learning".to_string(),
        model: aggregate
            .artifact_name
            .clone()
            .or_else(|| aggregate.routine_name.clone())
            .unwrap_or_else(|| "negative pattern".to_string()),
        route_key: None,
        logical_role: None,
        opportunity_type: aggregate.kind,
        summary,
        gpu_requirement: outcome_gpu_requirement(aggregate.kind),
        suggested_preset: outcome_preset(aggregate.kind),
        linked_target_id: aggregate.linked_target_id,
        source: Some("outcome_learning".to_string()),
        confidence: Some((0.45 + aggregate.count.min(5) as f64 * 0.1).clamp(0.45, 0.95)),
        signals,
        project_hint: Some(project_hint),
        metadata: serde_json::json!({
            "rank_score": aggregate.rank_score,
            "negative_outcome_count": aggregate.count,
            "pattern_key": aggregate.pattern_key,
            "contract_type": aggregate.contract_type,
            "artifact_type": aggregate.artifact_type,
            "artifact_name": aggregate.artifact_name,
            "routine_id": aggregate.routine_id,
            "routine_name": aggregate.routine_name,
        }),
        created_at: aggregate.first_seen,
        updated_at: aggregate.last_seen,
    }
}

fn outcome_summary_and_project_hint(
    aggregate: &OutcomeOpportunityAggregate,
    default_prompt_asset: &str,
) -> (String, serde_json::Value) {
    match aggregate.kind {
        ExperimentTargetKind::PromptAsset => {
            let target = aggregate
                .artifact_name
                .clone()
                .unwrap_or_else(|| default_prompt_asset.to_string());
            (
                format!(
                    "Use repeated negative outcome signals to benchmark and improve prompt behavior for {}.",
                    target
                ),
                serde_json::json!({
                    "name": format!("Outcome prompt benchmark for {}", target),
                    "mutable_paths": [target],
                    "fixed_paths": ["README.md"],
                    "metric_name": "outcome_success_rate",
                    "comparator": "higher_is_better",
                    "strategy": "Use the repeated negative outcome pattern as a benchmark seed, improve the prompt surface conservatively, and compare against the current baseline."
                }),
            )
        }
        ExperimentTargetKind::ToolPolicy => {
            let label = aggregate
                .routine_name
                .clone()
                .or_else(|| aggregate.artifact_name.clone())
                .unwrap_or_else(|| "tool orchestration".to_string());
            (
                format!(
                    "Investigate repeated negative outcome signals around {} and refine orchestration or notification policy.",
                    label
                ),
                serde_json::json!({
                    "name": format!("Outcome orchestration benchmark for {}", label),
                    "mutable_paths": ["src/agent/routine_engine.rs", "src/agent/outcomes.rs"],
                    "fixed_paths": ["README.md"],
                    "metric_name": "negative_outcome_rate",
                    "comparator": "lower_is_better",
                    "strategy": "Reduce repeated negative outcome patterns without broadening scope, and keep operator-facing behavior benchmarkable."
                }),
            )
        }
        ExperimentTargetKind::TrainingCode => (
            "Promote repeated negative durability signals into a benchmarked code-improvement search.".to_string(),
            serde_json::json!({
                "name": "Outcome-driven code benchmark",
                "mutable_paths": aggregate.artifact_name.clone().map(|value| vec![value]).unwrap_or_default(),
                "fixed_paths": ["README.md"],
                "metric_name": "regression_rate",
                "comparator": "lower_is_better",
                "strategy": "Use repeated negative durability outcomes as the seed benchmark and only mutate the code surface implicated by the pattern."
            }),
        ),
        kind => (
            format!(
                "Use repeated negative outcome signals to drive a focused {:?} benchmark.",
                kind
            ),
            serde_json::json!({
                "name": format!("Outcome-driven {:?} benchmark", kind),
                "mutable_paths": [],
                "fixed_paths": ["README.md"],
                "metric_name": "outcome_success_rate",
                "comparator": "higher_is_better",
                "strategy": "Turn repeated negative outcome evidence into a repeatable benchmark and search only the target surface."
            }),
        ),
    }
}

fn outcome_signals(aggregate: &OutcomeOpportunityAggregate) -> Vec<String> {
    let mut signals = vec![
        "outcome-backed evidence".to_string(),
        format!(
            "{} negative outcome{}",
            aggregate.count,
            if aggregate.count == 1 { "" } else { "s" }
        ),
    ];
    if let Some(artifact_name) = aggregate.artifact_name.as_deref() {
        signals.push(format!("target {}", artifact_name));
    }
    if let Some(routine_name) = aggregate.routine_name.as_deref() {
        signals.push(format!("routine {}", routine_name));
    }
    signals
}

fn outcome_gpu_requirement(kind: ExperimentTargetKind) -> ExperimentGpuRequirement {
    match kind {
        ExperimentTargetKind::TrainingCode | ExperimentTargetKind::TrainingConfig => {
            ExperimentGpuRequirement::Required
        }
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig => {
            ExperimentGpuRequirement::Recommended
        }
        _ => ExperimentGpuRequirement::NotNeeded,
    }
}

fn outcome_preset(kind: ExperimentTargetKind) -> ExperimentPreset {
    match kind {
        ExperimentTargetKind::PromptAsset | ExperimentTargetKind::RoutingPolicy => {
            ExperimentPreset::HostedPromptRouting
        }
        ExperimentTargetKind::RagConfig => ExperimentPreset::RagPipeline,
        ExperimentTargetKind::ToolPolicy => ExperimentPreset::ToolOrchestration,
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig => {
            ExperimentPreset::OpenWeightsInferenceTuning
        }
        ExperimentTargetKind::TrainingConfig => ExperimentPreset::SelfHostedFinetune,
        ExperimentTargetKind::TrainingCode => ExperimentPreset::OpenWeightsTrainingCode,
        ExperimentTargetKind::Evaluator | ExperimentTargetKind::Parser => {
            ExperimentPreset::AutoresearchSingleFile
        }
    }
}
