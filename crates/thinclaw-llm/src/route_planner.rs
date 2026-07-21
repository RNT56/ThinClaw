//! Unified route planner for all routing modes.
//!
//! `RoutePlanner` is the single routing engine. A single `RoutePlanner::plan()`
//! call (consumed by the runtime route-resolution path) supports four
//! strategies and owns CheapSplit cascade escalation directly — there is no
//! separate routing decorator in the provider chain:
//!
//! - **PrimaryOnly** — all requests → primary model
//! - **CheapSplit** — classify by complexity; the runtime runs the cheap lane and
//!   inspect-and-escalates to primary on uncertain cheap responses (cascade)
//! - **AdvisorExecutor** — executor runs everything, auto-escalates on risky and complex turns, and can consult the advisor on demand
//! - **Policy** — delegated to `RoutingPolicy` rule engine
//!
//! Reference: <https://claude.com/blog/the-advisor-strategy>

use std::collections::HashMap;

use thinclaw_llm_core::routing_policy::{
    ProviderCapabilitiesMetadata, RouteCandidate, RoutingContext, RoutingPolicy,
};
use thinclaw_llm_core::smart_routing::{SmartRoutingConfig, TaskComplexity, classify_message};
use thinclaw_settings::RoutingMode;

// ──────────────────────────────────────────────────────────────────────
// Core types (Phase 1)
// ──────────────────────────────────────────────────────────────────────

/// What this request needs from a provider.
#[derive(Debug, Clone, Default)]
pub struct RequiredCapabilities {
    pub streaming: bool,
    pub tool_use: bool,
    pub vision: bool,
    pub extended_thinking: bool,
}

impl RequiredCapabilities {
    /// Derive capabilities from an existing `RoutingContext`.
    pub fn from_routing_context(ctx: &RoutingContext) -> Self {
        Self {
            streaming: ctx.requires_streaming,
            tool_use: ctx.has_tools,
            vision: ctx.has_vision,
            extended_thinking: false,
        }
    }
}

/// Provider capability metadata returned by `LlmProvider::capabilities()`.
#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    pub supports_streaming: Option<bool>,
    pub supports_tools: Option<bool>,
    pub supports_vision: Option<bool>,
    pub supports_thinking: Option<bool>,
    pub max_context_tokens: Option<u32>,
}

impl ProviderCapabilities {
    fn from_candidate(metadata: &ProviderCapabilitiesMetadata) -> Self {
        Self {
            supports_streaming: metadata.supports_streaming,
            supports_tools: metadata.supports_tools,
            supports_vision: metadata.supports_vision,
            supports_thinking: metadata.supports_thinking,
            max_context_tokens: metadata.max_context_tokens,
        }
    }
}

/// Weighted score breakdown for a candidate route.
#[derive(Debug, Clone)]
pub struct RoutingScoreBreakdown {
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
    pub policy_bias: f64,
    pub composite: f64,
}

#[derive(Debug, Clone)]
pub struct CandidateScore {
    pub target: String,
    pub telemetry_key: Option<String>,
    pub breakdown: RoutingScoreBreakdown,
}

#[derive(Debug, Clone)]
pub struct CandidateRejection {
    pub target: String,
    pub reason: String,
}

/// Weights for score dimensions.
#[derive(Debug, Clone)]
pub struct RoutingWeights {
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
}

impl Default for RoutingWeights {
    fn default() -> Self {
        Self {
            quality: 0.3,
            cost: 0.3,
            latency: 0.2,
            health: 0.2,
        }
    }
}

/// Input to the planner.
#[derive(Debug, Clone)]
pub struct RoutePlannerInput {
    pub required_capabilities: RequiredCapabilities,
    pub routing_mode: RoutingMode,
    pub routing_context: RoutingContext,
    /// Explicit model override from request or conversation.
    pub model_override: Option<String>,
    /// Current provider health state (target → 0.0–1.0).
    pub provider_health: HashMap<String, f64>,
    /// Available routing targets (from runtime manager).
    pub candidates: Vec<RouteCandidate>,
    /// Cost accumulated in this agent turn so far (USD).
    pub turn_cost_usd: f64,
    /// Current daily budget utilization (0.0–1.0), if budget configured.
    pub budget_utilization: Option<f64>,
    /// The last user message text (for CheapSplit classification).
    pub last_user_message: Option<String>,
    /// Optional advisor escalation prompt override (AdvisorExecutor mode).
    pub advisor_escalation_prompt: Option<String>,
    /// Ordered primary-provider preferences derived from user settings.
    pub primary_provider_preferences: Vec<String>,
    /// Ordered cheap-provider preferences derived from user settings.
    pub cheap_provider_preferences: Vec<String>,
}

/// How to handle post-response quality escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadePolicy {
    /// No cascade — use result as-is.
    None,
    /// Inspect response for uncertainty; escalate to first fallback if uncertain.
    InspectAndEscalate,
}

/// Advisor configuration for AdvisorExecutor mode.
#[derive(Debug, Clone)]
pub struct AdvisorConfig {
    /// The advisor model target (e.g., "primary").
    pub advisor_target: String,
    /// Maximum advisor consultations per agent turn.
    pub max_advisor_calls: u32,
    /// System prompt for the advisor.
    pub advisor_system_prompt: String,
}

/// Output from the planner.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    /// Primary target (e.g., "primary", "cheap", "openai/gpt-4o").
    pub target: String,
    /// Ordered fallbacks.
    pub fallbacks: Vec<String>,
    /// Why this target was selected.
    pub reason: String,
    /// Score breakdown for observability (None for override/simple paths).
    pub score: Option<RoutingScoreBreakdown>,
    /// Candidate targets considered for this decision.
    pub candidate_list: Vec<String>,
    /// Candidates hard-rejected by capability/context gates.
    pub rejections: Vec<CandidateRejection>,
    /// Per-candidate score breakdown for explainability.
    pub score_breakdown: Vec<CandidateScore>,
    /// Planner diagnostics (including fail-open capability notes).
    pub diagnostics: Vec<String>,
    /// Canonical telemetry key: "logical_role|provider_slug|model_id".
    pub telemetry_key: String,
    /// Index of the matched policy rule, if any.
    pub matched_rule_index: Option<usize>,
    /// Post-response cascade behavior.
    pub cascade: CascadePolicy,
    /// Advisor configuration (AdvisorExecutor mode only).
    pub advisor: Option<AdvisorConfig>,
    /// Whether the advisor lane is usable for this decision.
    pub advisor_ready: bool,
    /// Why the advisor lane is disabled, if applicable.
    pub advisor_disabled_reason: Option<String>,
    /// Concrete executor target selected for this decision, when applicable.
    pub executor_target: Option<String>,
    /// Concrete advisor target selected for this decision, when applicable.
    pub advisor_target: Option<String>,
    /// Whether two-phase tool synthesis is recommended.
    pub tool_phase_synthesis: bool,
}

impl RouteDecision {
    fn primary(reason: impl Into<String>) -> Self {
        Self {
            target: "primary".to_string(),
            fallbacks: Vec::new(),
            reason: reason.into(),
            score: None,
            candidate_list: Vec::new(),
            rejections: Vec::new(),
            score_breakdown: Vec::new(),
            diagnostics: Vec::new(),
            telemetry_key: "primary||".to_string(),
            matched_rule_index: None,
            cascade: CascadePolicy::None,
            advisor: None,
            advisor_ready: false,
            advisor_disabled_reason: None,
            executor_target: None,
            advisor_target: None,
            tool_phase_synthesis: false,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Latency source trait (Phase 3)
// ──────────────────────────────────────────────────────────────────────

/// Abstraction over latency data for scoring.
pub trait LatencySource: Send + Sync {
    fn latency_p50(&self, target: &str) -> Option<f64>;
}

/// No-op latency source for when no tracker is available.
pub struct NoopLatencySource;

impl LatencySource for NoopLatencySource {
    fn latency_p50(&self, _target: &str) -> Option<f64> {
        None
    }
}

// ──────────────────────────────────────────────────────────────────────
// Health probe trait (Phase 8)
// ──────────────────────────────────────────────────────────────────────

/// Abstraction for provider health data.
pub trait HealthProbe: Send + Sync {
    fn health_score(&self) -> f64;
    fn probe_target(&self) -> &str;
}

// ──────────────────────────────────────────────────────────────────────
// Structured scorer (Phase 3)
// ──────────────────────────────────────────────────────────────────────

/// Multi-dimensional scorer for routing candidates.
pub struct RouteScorer {
    weights: RoutingWeights,
}

#[derive(Debug, Clone)]
pub enum ScoreOutcome {
    Scored(RoutingScoreBreakdown),
    Rejected(String),
}

impl RouteScorer {
    pub fn new(weights: RoutingWeights) -> Self {
        Self { weights }
    }

    /// Score a candidate. Returns `None` if the candidate fails a hard gate.
    #[allow(clippy::too_many_arguments)]
    pub fn score(
        &self,
        candidate: &RouteCandidate,
        capabilities: &ProviderCapabilities,
        required: &RequiredCapabilities,
        health: f64,
        latency_p50_ms: Option<f64>,
        policy_bias: f64,
        budget_utilization: Option<f64>,
        estimated_input_tokens: u32,
    ) -> ScoreOutcome {
        // ── Hard gates ─────────────────────────────────────────
        if required.streaming && capabilities.supports_streaming == Some(false) {
            return ScoreOutcome::Rejected("missing required capability: streaming".to_string());
        }
        if required.tool_use && capabilities.supports_tools == Some(false) {
            return ScoreOutcome::Rejected("missing required capability: tool_use".to_string());
        }
        if required.vision && capabilities.supports_vision == Some(false) {
            return ScoreOutcome::Rejected("missing required capability: vision".to_string());
        }
        if required.extended_thinking && capabilities.supports_thinking == Some(false) {
            return ScoreOutcome::Rejected(
                "missing required capability: extended_thinking".to_string(),
            );
        }

        // Context window gate
        if let Some(max_ctx) = capabilities.max_context_tokens
            && estimated_input_tokens > max_ctx
        {
            return ScoreOutcome::Rejected(format!(
                "context overflow: {} > {} tokens",
                estimated_input_tokens, max_ctx
            ));
        }

        // ── Dimension scores ───────────────────────────────────
        let quality = quality_score_for_candidate(candidate, capabilities);
        let mut cost = cost_score(candidate.cost_per_m_usd);
        if candidate.cost_stale {
            // Penalize stale dynamic pricing so fresh-priced candidates win ties.
            cost *= 0.75;
        }
        let latency = latency_score(candidate.latency_p50_ms.or(latency_p50_ms));

        // ── Budget-aware cost pressure ─────────────────────────
        let cost_weight = match budget_utilization {
            Some(u) if u > 0.9 => self.weights.cost * 3.0,
            Some(u) if u > 0.8 => self.weights.cost * 2.0,
            Some(u) if u > 0.5 => self.weights.cost * 1.5,
            _ => self.weights.cost,
        };

        let composite = self.weights.quality * quality
            + cost_weight * cost
            + self.weights.latency * latency
            + self.weights.health * health
            + policy_bias;

        ScoreOutcome::Scored(RoutingScoreBreakdown {
            quality,
            cost,
            latency,
            health,
            policy_bias,
            composite,
        })
    }
}

fn quality_score_for_candidate(
    candidate: &RouteCandidate,
    capabilities: &ProviderCapabilities,
) -> f64 {
    if let Some(compat) = candidate_model_compat(candidate) {
        return compat.routing_quality_score();
    }

    thinclaw_config::model_compat::estimate_routing_quality(
        capabilities.supports_streaming,
        capabilities.supports_tools,
        capabilities.supports_vision,
        capabilities.supports_thinking,
        None,
        None,
        capabilities.max_context_tokens,
        None,
        candidate.cost_per_m_usd,
    )
}

fn candidate_model_compat(
    candidate: &RouteCandidate,
) -> Option<thinclaw_config::model_compat::ModelCompat> {
    candidate
        .model_id
        .as_deref()
        .or_else(|| candidate.target.rsplit_once('/').map(|(_, model)| model))
        .and_then(thinclaw_config::model_compat::find_model)
}

/// Cost score: cheaper is higher (inverted, normalized 0–1).
fn cost_score(cost_per_m_usd: Option<f64>) -> f64 {
    match cost_per_m_usd {
        Some(c) if c > 0.0 => {
            // Normalize: $0/M → 1.0, $100/M → ~0.0
            1.0 / (1.0 + c / 10.0)
        }
        Some(_) => 1.0, // free or zero cost
        None => 0.5,    // unknown cost
    }
}

/// Latency score: lower latency is higher (inverted, normalized 0–1).
fn latency_score(p50_ms: Option<f64>) -> f64 {
    match p50_ms {
        Some(ms) if ms > 0.0 => {
            // Normalize: 0ms → 1.0, 10000ms → ~0.0
            1.0 / (1.0 + ms / 2000.0)
        }
        Some(_) => 1.0,
        None => 0.5, // unknown latency
    }
}

// ──────────────────────────────────────────────────────────────────────
// RoutePlanner (Phase 4)
// ──────────────────────────────────────────────────────────────────────

/// Advisor system prompt template.
pub const ADVISOR_SYSTEM_PROMPT: &str = "\
You are an advisor to a less capable model that is executing a task. \
Your role is to provide strategic guidance, NOT to execute the task yourself.\n\
\n\
Act like a careful senior reviewer. Prefer preventing bad execution over optimistic continuation. \
If the executor is missing critical context, stuck, or following a risky plan, say so clearly.\n\
\n\
Respond with:\n\
- A clear plan or recommendation\n\
- Specific reasoning for your recommendation\n\
- Any corrections to the executor's approach\n\
- A STOP signal if the executor should abandon the current approach\n\
\n\
Keep your guidance concise (under 500 words). Do not produce user-facing \
output or call tools — the executor handles all execution.";

/// Unified route planner that handles all routing modes.
pub struct RoutePlanner {
    /// Scorer infrastructure — ready for scored routing with live health/cost signals.
    /// Currently used in tests and will be promoted when scored mode replaces keyword classification.
    #[allow(dead_code)]
    scorer: RouteScorer,
    cheap_split_config: SmartRoutingConfig,
    cascade_enabled: bool,
    tool_phase_synthesis_enabled: bool,
    advisor_max_calls: u32,
}

impl RoutePlanner {
    pub fn new(
        cascade_enabled: bool,
        tool_phase_synthesis_enabled: bool,
        advisor_max_calls: u32,
    ) -> Self {
        Self {
            scorer: RouteScorer::new(RoutingWeights::default()),
            cheap_split_config: SmartRoutingConfig::default(),
            cascade_enabled,
            tool_phase_synthesis_enabled,
            advisor_max_calls,
        }
    }

    /// Update configuration from provider settings (called on hot-reload).
    pub fn update_config(
        &mut self,
        cascade_enabled: bool,
        tool_phase_synthesis_enabled: bool,
        advisor_max_calls: u32,
    ) {
        self.cascade_enabled = cascade_enabled;
        self.tool_phase_synthesis_enabled = tool_phase_synthesis_enabled;
        self.advisor_max_calls = advisor_max_calls;
    }

    /// Produce a routing decision.
    ///
    /// Precedence (strict order):
    /// 1. Explicit model override → bypass scoring
    /// 2. Mode-specific logic (PrimaryOnly / CheapSplit / AdvisorExecutor / Policy)
    /// 3. Fallback chain
    pub fn plan(&self, input: &RoutePlannerInput, policy: Option<&RoutingPolicy>) -> RouteDecision {
        // ── 1. Explicit override ───────────────────────────────
        if let Some(ref model_override) = input.model_override {
            return RouteDecision {
                target: model_override.clone(),
                fallbacks: Vec::new(),
                reason: format!("Explicit model override: {}", model_override),
                score: None,
                candidate_list: input.candidates.iter().map(|c| c.target.clone()).collect(),
                rejections: Vec::new(),
                score_breakdown: Vec::new(),
                diagnostics: Vec::new(),
                telemetry_key: format!("override||{}", model_override),
                matched_rule_index: None,
                cascade: CascadePolicy::None,
                advisor: None,
                advisor_ready: false,
                advisor_disabled_reason: None,
                executor_target: None,
                advisor_target: None,
                tool_phase_synthesis: false,
            };
        }

        // ── 2. Mode-specific logic ─────────────────────────────
        match input.routing_mode {
            RoutingMode::PrimaryOnly => self.plan_primary_only(input),
            RoutingMode::CheapSplit => self.plan_cheap_split(input),
            RoutingMode::AdvisorExecutor => self.plan_advisor_executor(input),
            RoutingMode::Policy => self.plan_policy(input, policy),
        }
    }

    // -- PrimaryOnly --

    fn plan_primary_only(&self, input: &RoutePlannerInput) -> RouteDecision {
        let mut decision = RouteDecision::primary("PrimaryOnly mode");
        decision.candidate_list = input.candidates.iter().map(|c| c.target.clone()).collect();
        decision
    }

    // -- CheapSplit (preserved) --

    fn plan_cheap_split(&self, input: &RoutePlannerInput) -> RouteDecision {
        // Hard override: tools/streaming → always primary
        if input.required_capabilities.tool_use || input.required_capabilities.streaming {
            let mut decision =
                RouteDecision::primary("CheapSplit: tool/streaming request → primary (always)");
            decision.telemetry_key = "primary||".to_string();
            decision.candidate_list = input.candidates.iter().map(|c| c.target.clone()).collect();

            // Tool-phase synthesis decision
            if input.required_capabilities.tool_use
                && self.tool_phase_synthesis_enabled
                && input.model_override.is_none()
                && input.candidates.iter().any(|candidate| {
                    candidate.target == "cheap" || candidate.target.ends_with("@cheap")
                })
            {
                decision.tool_phase_synthesis = true;
            }

            return decision;
        }

        let complexity = self.derive_cheap_split_complexity(input);
        let (bias_cheap, bias_primary, reason) = match complexity {
            TaskComplexity::Simple => (
                0.25,
                -0.08,
                "CheapSplit: simple context/profile favors cheap model",
            ),
            TaskComplexity::Complex => (
                -0.08,
                0.25,
                "CheapSplit: complex context/profile favors primary model",
            ),
            TaskComplexity::Moderate => (
                0.12,
                0.08,
                if self.cascade_enabled {
                    "CheapSplit: moderate context/profile favors cheap with cascade"
                } else {
                    "CheapSplit: moderate context/profile favors cheap without cascade"
                },
            ),
        };

        let evaluation = self.evaluate_candidates(
            input,
            |candidate| {
                if candidate.target == "cheap" || candidate.target.ends_with("@cheap") {
                    bias_cheap
                } else if candidate.target == "primary" || candidate.target.ends_with("@primary") {
                    bias_primary
                } else {
                    0.0
                }
            },
            Some(&["cheap", "primary"]),
        );

        let cascade = if complexity == TaskComplexity::Moderate && self.cascade_enabled {
            CascadePolicy::InspectAndEscalate
        } else {
            CascadePolicy::None
        };

        let selected = evaluation
            .ranked
            .first()
            .or_else(|| evaluation.ranked_all.first());
        if let Some(selected) = selected {
            let mut fallbacks = Vec::new();
            if selected.target == "cheap" {
                fallbacks.push("primary".to_string());
            }
            if selected.target != "primary" && !fallbacks.iter().any(|fb| fb == "primary") {
                fallbacks.push("primary".to_string());
            }

            return RouteDecision {
                target: selected.target.clone(),
                fallbacks,
                reason: reason.to_string(),
                score: Some(selected.breakdown.clone()),
                candidate_list: evaluation.candidate_list,
                rejections: evaluation.rejections,
                score_breakdown: evaluation.score_breakdown,
                diagnostics: evaluation.diagnostics,
                telemetry_key: selected
                    .telemetry_key
                    .clone()
                    .unwrap_or_else(|| selected.target.clone()),
                matched_rule_index: None,
                cascade,
                advisor: None,
                advisor_ready: false,
                advisor_disabled_reason: None,
                executor_target: None,
                advisor_target: None,
                tool_phase_synthesis: false,
            };
        }

        let mut decision = RouteDecision::primary(reason);
        decision.candidate_list = evaluation.candidate_list;
        decision.rejections = evaluation.rejections;
        decision.score_breakdown = evaluation.score_breakdown;
        decision.diagnostics = evaluation.diagnostics;
        if !decision
            .diagnostics
            .iter()
            .any(|d| d.contains("NO_CAPABLE_CANDIDATE"))
        {
            decision
                .diagnostics
                .push("NO_CAPABLE_CANDIDATE: all candidates hard-rejected".to_string());
        }
        decision
    }

    // -- AdvisorExecutor (new) --

    fn plan_advisor_executor(&self, input: &RoutePlannerInput) -> RouteDecision {
        let evaluation = self.evaluate_candidates(
            input,
            |candidate| advisor_executor_lane_bias(candidate, input),
            None,
        );
        let executor = evaluation
            .ranked
            .iter()
            .find(|candidate| candidate.is_executor_lane())
            .cloned();
        let executor_identity =
            preferred_lane_identity_candidate(&evaluation.ranked, PreferredLaneRole::Cheap)
                .or_else(|| executor.clone());
        let executor = executor.or_else(|| executor_identity.clone());
        let request_primary = evaluation
            .ranked
            .iter()
            .find(|candidate| candidate.is_primary_lane())
            .cloned();

        let mut advisor_input = input.clone();
        advisor_input.required_capabilities = RequiredCapabilities {
            streaming: false,
            tool_use: false,
            vision: false,
            extended_thinking: false,
        };
        let advisor_evaluation = self.evaluate_candidates(
            &advisor_input,
            |candidate| primary_lane_bias(candidate, &input.primary_provider_preferences),
            Some(&["primary"]),
        );
        let advisor = advisor_evaluation.ranked.first().cloned();
        let advisor_identity = preferred_lane_identity_candidate(
            &advisor_evaluation.ranked,
            PreferredLaneRole::Primary,
        )
        .or_else(|| advisor.clone());
        let advisor = advisor.or_else(|| advisor_identity.clone());

        let primary_fallback = request_primary
            .clone()
            .or_else(|| advisor_identity.clone())
            .or_else(|| {
                evaluation
                    .ranked_all
                    .iter()
                    .find(|candidate| candidate.is_primary_lane())
                    .cloned()
            });

        let combined_candidate_list = evaluation.candidate_list.clone();
        let mut combined_rejections = evaluation.rejections.clone();
        combined_rejections.extend(advisor_evaluation.rejections.iter().cloned().map(
            |mut rejection| {
                rejection.reason = format!("advisor lane: {}", rejection.reason);
                rejection
            },
        ));
        let mut combined_score_breakdown = evaluation.score_breakdown.clone();
        let advisor_only_scores = advisor_evaluation
            .score_breakdown
            .iter()
            .filter(|score| {
                !combined_score_breakdown
                    .iter()
                    .any(|existing| existing.target == score.target)
            })
            .cloned()
            .collect::<Vec<_>>();
        combined_score_breakdown.extend(advisor_only_scores);
        let mut combined_diagnostics = evaluation.diagnostics.clone();
        combined_diagnostics.extend(advisor_evaluation.diagnostics.clone());
        combined_diagnostics.sort();
        combined_diagnostics.dedup();

        let disable_reason = match (&executor_identity, &advisor_identity) {
            (None, _) => {
                Some("no cheap-capable executor route satisfies the current request".to_string())
            }
            (Some(_), None) => {
                Some("no primary advisor route is available for consultation".to_string())
            }
            (Some(executor), Some(advisor)) if executor.same_identity(advisor) => Some(
                "advisor target resolves to the same provider/model as the executor".to_string(),
            ),
            _ => None,
        };

        if let Some(reason) = disable_reason {
            combined_diagnostics.push(format!("ADVISOR_DISABLED: {}", reason));
            let fallback = primary_fallback.unwrap_or_else(|| ScoredCandidate {
                target: "primary".to_string(),
                telemetry_key: Some("primary||".to_string()),
                provider_slug: None,
                model_id: None,
                breakdown: RoutingScoreBreakdown {
                    quality: 0.0,
                    cost: 0.0,
                    latency: 0.0,
                    health: 0.0,
                    policy_bias: 0.0,
                    composite: 0.0,
                },
            });
            return RouteDecision {
                target: fallback.target.clone(),
                fallbacks: Vec::new(),
                reason: format!("AdvisorExecutor degraded to primary-only: {}", reason),
                score: Some(fallback.breakdown.clone()),
                candidate_list: combined_candidate_list,
                rejections: combined_rejections,
                score_breakdown: combined_score_breakdown,
                diagnostics: combined_diagnostics,
                telemetry_key: fallback
                    .telemetry_key
                    .clone()
                    .unwrap_or_else(|| "primary||".to_string()),
                matched_rule_index: None,
                cascade: CascadePolicy::None,
                advisor: None,
                advisor_ready: false,
                advisor_disabled_reason: Some(reason),
                executor_target: executor_identity
                    .as_ref()
                    .map(|candidate| candidate.target.clone())
                    .or_else(|| Some(fallback.target.clone())),
                advisor_target: advisor_identity
                    .as_ref()
                    .map(|candidate| candidate.target.clone()),
                tool_phase_synthesis: false,
            };
        }

        let (Some(executor), Some(advisor)) = (executor, advisor) else {
            let mut fallback = RouteDecision::primary(
                "AdvisorExecutor degraded because a selected lane disappeared",
            );
            fallback.candidate_list = combined_candidate_list;
            fallback.rejections = combined_rejections;
            fallback.score_breakdown = combined_score_breakdown;
            fallback.diagnostics = combined_diagnostics;
            fallback
                .diagnostics
                .push("ADVISOR_DISABLED: selected lane disappeared".to_string());
            return fallback;
        };
        let advisor_target = advisor.target.clone();
        let executor_identity_target = executor_identity
            .as_ref()
            .map(|candidate| candidate.target.clone())
            .unwrap_or_else(|| executor.target.clone());
        let advisor_identity_target = advisor_identity
            .as_ref()
            .map(|candidate| candidate.target.clone())
            .unwrap_or_else(|| advisor_target.clone());

        RouteDecision {
            target: executor.target.clone(),
            fallbacks: vec![advisor_target.clone()],
            reason: "AdvisorExecutor: executor lane handles the request and may consult the advisor lane".to_string(),
            score: Some(executor.breakdown.clone()),
            candidate_list: combined_candidate_list,
            rejections: combined_rejections,
            score_breakdown: combined_score_breakdown,
            diagnostics: combined_diagnostics,
            telemetry_key: executor
                .telemetry_key
                .clone()
                .unwrap_or_else(|| "executor||".to_string()),
            matched_rule_index: None,
            cascade: CascadePolicy::None,
            advisor: Some(AdvisorConfig {
                advisor_target: advisor_target.clone(),
                max_advisor_calls: self.advisor_max_calls,
                advisor_system_prompt: input
                    .advisor_escalation_prompt
                    .clone()
                    .filter(|prompt| !prompt.trim().is_empty())
                    .unwrap_or_else(|| ADVISOR_SYSTEM_PROMPT.to_string()),
            }),
            advisor_ready: true,
            advisor_disabled_reason: None,
            executor_target: Some(executor_identity_target),
            advisor_target: Some(advisor_identity_target),
            tool_phase_synthesis: false,
        }
    }

    // -- Policy --

    fn plan_policy(
        &self,
        input: &RoutePlannerInput,
        policy: Option<&RoutingPolicy>,
    ) -> RouteDecision {
        let Some(policy) = policy else {
            return RouteDecision::primary("Policy mode but no policy configured");
        };

        let decision = policy.select_decision(&input.routing_context, &input.candidates);
        let evaluation = self.evaluate_candidates(input, |_candidate| 0.0, None);

        let selected_from_policy = evaluation
            .ranked_all
            .iter()
            .find(|ranked| ranked.target == decision.target);
        let fallback_selection = evaluation.ranked_all.first();

        let (selected, reason) = if let Some(selected) = selected_from_policy {
            let reason = decision
                .matched_rule_index
                .map(|idx| format!("Policy rule {} matched", idx))
                .unwrap_or_else(|| "Policy default target".to_string());
            (selected, reason)
        } else if let Some(selected) = fallback_selection {
            let mut reason = decision
                .matched_rule_index
                .map(|idx| {
                    format!(
                        "Policy rule {} matched but target unavailable/capability-rejected",
                        idx
                    )
                })
                .unwrap_or_else(|| {
                    "Policy default target unavailable; scorer tie-break".to_string()
                });
            reason.push_str("; selected highest scored capable candidate");
            (selected, reason)
        } else {
            let mut fallback = RouteDecision::primary("Policy mode: no capable candidates");
            fallback.candidate_list = evaluation.candidate_list;
            fallback.rejections = evaluation.rejections;
            fallback.score_breakdown = evaluation.score_breakdown;
            fallback.diagnostics = evaluation.diagnostics;
            if !fallback
                .diagnostics
                .iter()
                .any(|d| d.contains("NO_CAPABLE_CANDIDATE"))
            {
                fallback
                    .diagnostics
                    .push("NO_CAPABLE_CANDIDATE: all candidates hard-rejected".to_string());
            }
            fallback.matched_rule_index = decision.matched_rule_index;
            return fallback;
        };

        RouteDecision {
            target: selected.target.clone(),
            fallbacks: decision.fallbacks,
            reason,
            score: Some(selected.breakdown.clone()),
            candidate_list: evaluation.candidate_list,
            rejections: evaluation.rejections,
            score_breakdown: evaluation.score_breakdown,
            diagnostics: evaluation.diagnostics,
            telemetry_key: selected.telemetry_key.clone().unwrap_or_else(|| {
                decision
                    .matched_rule_index
                    .map(|idx| format!("policy_rule_{}||", idx))
                    .unwrap_or_else(|| "policy_default||".to_string())
            }),
            matched_rule_index: decision.matched_rule_index,
            cascade: CascadePolicy::None,
            advisor: None,
            advisor_ready: false,
            advisor_disabled_reason: None,
            executor_target: None,
            advisor_target: None,
            tool_phase_synthesis: false,
        }
    }

    // -- Helper --

    fn derive_cheap_split_complexity(&self, input: &RoutePlannerInput) -> TaskComplexity {
        // Runtime context first: avoid empty-message defaults.
        let mut base = if input.routing_context.has_vision
            || input.routing_context.estimated_input_tokens >= 12_000
        {
            TaskComplexity::Complex
        } else if input.routing_context.estimated_input_tokens <= 600
            && !input.routing_context.has_tools
            && !input.routing_context.requires_streaming
        {
            TaskComplexity::Simple
        } else {
            TaskComplexity::Moderate
        };

        // Optional enrichment from last user message (only when non-empty).
        if let Some(msg) = input
            .last_user_message
            .as_deref()
            .map(str::trim)
            .filter(|msg| !msg.is_empty())
        {
            let text_complexity = classify_message(msg, &self.cheap_split_config);
            base = merge_complexity(base, text_complexity);
        }

        base
    }

    fn evaluate_candidates<F>(
        &self,
        input: &RoutePlannerInput,
        policy_bias_for: F,
        target_filter: Option<&[&str]>,
    ) -> CandidateEvaluation
    where
        F: Fn(&RouteCandidate) -> f64,
    {
        let mut ranked_all = Vec::new();
        let mut ranked_filtered = Vec::new();
        let mut rejections = Vec::new();
        let mut score_breakdown = Vec::new();
        let mut diagnostics = Vec::new();
        let candidate_list: Vec<String> =
            input.candidates.iter().map(|c| c.target.clone()).collect();

        for candidate in &input.candidates {
            let capabilities = ProviderCapabilities::from_candidate(&candidate.capabilities);
            let missing_required_capabilities =
                missing_required_capability_labels(&input.required_capabilities, &capabilities);
            if !missing_required_capabilities.is_empty() {
                let joined = missing_required_capabilities.join(", ");
                let is_executor_lane =
                    candidate.target == "cheap" || candidate.target.ends_with("@cheap");
                diagnostics.push(format!(
                    "Capability metadata unknown ({joined}) for '{}'; {}",
                    candidate.target,
                    if is_executor_lane {
                        "rejecting executor lane"
                    } else {
                        "retaining primary fail-open fallback"
                    }
                ));
                if is_executor_lane {
                    rejections.push(CandidateRejection {
                        target: candidate.target.clone(),
                        reason: format!(
                            "missing verified capability metadata for executor lane: {joined}"
                        ),
                    });
                    continue;
                }
            }

            let health = candidate
                .health
                .or_else(|| {
                    candidate
                        .telemetry_key
                        .as_ref()
                        .and_then(|key| input.provider_health.get(key).copied())
                })
                .or_else(|| input.provider_health.get(&candidate.target).copied())
                .unwrap_or(0.8);
            let latency = candidate.latency_p50_ms;
            let policy_bias = policy_bias_for(candidate);
            match self.scorer.score(
                candidate,
                &capabilities,
                &input.required_capabilities,
                health,
                latency,
                policy_bias,
                input.budget_utilization,
                input.routing_context.estimated_input_tokens,
            ) {
                ScoreOutcome::Scored(breakdown) => {
                    let scored = ScoredCandidate {
                        target: candidate.target.clone(),
                        telemetry_key: candidate.telemetry_key.clone(),
                        provider_slug: candidate.provider_slug.clone(),
                        model_id: candidate.model_id.clone(),
                        breakdown: breakdown.clone(),
                    };
                    ranked_all.push(scored.clone());
                    let passes_filter = match target_filter {
                        None => true,
                        Some(filters) => filters.iter().any(|entry| {
                            if *entry == "cheap" {
                                candidate.target == "cheap" || candidate.target.ends_with("@cheap")
                            } else if *entry == "primary" {
                                candidate.target == "primary"
                                    || candidate.target.ends_with("@primary")
                            } else {
                                *entry == candidate.target
                            }
                        }),
                    };
                    if passes_filter {
                        ranked_filtered.push(scored.clone());
                    }
                    score_breakdown.push(CandidateScore {
                        target: candidate.target.clone(),
                        telemetry_key: candidate.telemetry_key.clone(),
                        breakdown,
                    });
                }
                ScoreOutcome::Rejected(reason) => {
                    rejections.push(CandidateRejection {
                        target: candidate.target.clone(),
                        reason,
                    });
                }
            }
        }

        ranked_all.sort_by(|a, b| {
            b.breakdown
                .composite
                .partial_cmp(&a.breakdown.composite)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked_filtered.sort_by(|a, b| {
            b.breakdown
                .composite
                .partial_cmp(&a.breakdown.composite)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        diagnostics.sort();
        diagnostics.dedup();
        if ranked_all.is_empty() && !candidate_list.is_empty() {
            diagnostics.push("NO_CAPABLE_CANDIDATE: all candidates hard-rejected".to_string());
        }

        CandidateEvaluation {
            candidate_list,
            rejections,
            score_breakdown,
            diagnostics,
            ranked: ranked_filtered,
            ranked_all,
        }
    }
}

#[derive(Debug, Clone)]
struct ScoredCandidate {
    target: String,
    telemetry_key: Option<String>,
    provider_slug: Option<String>,
    model_id: Option<String>,
    breakdown: RoutingScoreBreakdown,
}

impl ScoredCandidate {
    fn is_executor_lane(&self) -> bool {
        self.target == "cheap" || self.target.ends_with("@cheap")
    }

    fn is_primary_lane(&self) -> bool {
        self.target == "primary" || self.target.ends_with("@primary")
    }

    fn same_identity(&self, other: &Self) -> bool {
        match (
            self.provider_slug.as_deref(),
            self.model_id.as_deref(),
            other.provider_slug.as_deref(),
            other.model_id.as_deref(),
        ) {
            (Some(left_provider), Some(left_model), Some(right_provider), Some(right_model)) => {
                left_provider == right_provider && left_model == right_model
            }
            _ => self.target == other.target,
        }
    }
}

#[derive(Debug, Clone)]
struct CandidateEvaluation {
    candidate_list: Vec<String>,
    rejections: Vec<CandidateRejection>,
    score_breakdown: Vec<CandidateScore>,
    diagnostics: Vec<String>,
    ranked: Vec<ScoredCandidate>,
    ranked_all: Vec<ScoredCandidate>,
}

fn missing_required_capability_labels(
    required: &RequiredCapabilities,
    capabilities: &ProviderCapabilities,
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if required.streaming && capabilities.supports_streaming.is_none() {
        missing.push("streaming");
    }
    if required.tool_use && capabilities.supports_tools.is_none() {
        missing.push("tool_use");
    }
    if required.vision && capabilities.supports_vision.is_none() {
        missing.push("vision");
    }
    if required.extended_thinking && capabilities.supports_thinking.is_none() {
        missing.push("extended_thinking");
    }
    missing
}

#[derive(Debug, Clone, Copy)]
enum PreferredLaneRole {
    Primary,
    Cheap,
}

fn preferred_lane_identity_candidate(
    candidates: &[ScoredCandidate],
    role: PreferredLaneRole,
) -> Option<ScoredCandidate> {
    let matches_role = |candidate: &&ScoredCandidate| match role {
        PreferredLaneRole::Primary => candidate.is_primary_lane(),
        PreferredLaneRole::Cheap => candidate.is_executor_lane(),
    };

    candidates
        .iter()
        .find(|candidate| {
            matches_role(candidate)
                && candidate.target.ends_with(match role {
                    PreferredLaneRole::Primary => "@primary",
                    PreferredLaneRole::Cheap => "@cheap",
                })
        })
        .cloned()
        .or_else(|| candidates.iter().find(matches_role).cloned())
}

fn advisor_executor_lane_bias(candidate: &RouteCandidate, input: &RoutePlannerInput) -> f64 {
    if candidate.target == "cheap" || candidate.target.ends_with("@cheap") {
        return cheap_lane_bias(candidate, &input.cheap_provider_preferences);
    }
    if candidate.target == "primary" || candidate.target.ends_with("@primary") {
        return primary_lane_bias(candidate, &input.primary_provider_preferences);
    }
    0.0
}

fn primary_lane_bias(candidate: &RouteCandidate, preferences: &[String]) -> f64 {
    provider_preference_bias(candidate.provider_slug.as_deref(), preferences, 0.08)
}

fn cheap_lane_bias(candidate: &RouteCandidate, preferences: &[String]) -> f64 {
    provider_preference_bias(candidate.provider_slug.as_deref(), preferences, 0.10)
}

fn provider_preference_bias(
    provider_slug: Option<&str>,
    preferences: &[String],
    top_bias: f64,
) -> f64 {
    let Some(provider_slug) = provider_slug else {
        return 0.0;
    };
    let Some(index) = preferences.iter().position(|entry| entry == provider_slug) else {
        return 0.0;
    };
    match index {
        0 => top_bias,
        1 => top_bias * 0.5,
        2 => top_bias * 0.25,
        _ => 0.0,
    }
}

fn merge_complexity(a: TaskComplexity, b: TaskComplexity) -> TaskComplexity {
    match (a, b) {
        (TaskComplexity::Complex, _) | (_, TaskComplexity::Complex) => TaskComplexity::Complex,
        (TaskComplexity::Moderate, _) | (_, TaskComplexity::Moderate) => TaskComplexity::Moderate,
        _ => TaskComplexity::Simple,
    }
}

// ──────────────────────────────────────────────────────────────────────
// Config validation (Phase 7)
// ──────────────────────────────────────────────────────────────────────

/// Validate provider settings and return warnings.
pub fn validate_providers_settings(settings: &thinclaw_settings::ProvidersSettings) -> Vec<String> {
    let mut warnings = Vec::new();

    // AdvisorExecutor requires a cheap model (executor)
    if settings.routing_mode == RoutingMode::AdvisorExecutor
        && settings.cheap_model.is_none()
        && settings.preferred_cheap_provider.is_none()
    {
        warnings.push(
            "AdvisorExecutor mode requires a cheap model (executor). \
             Configure a cheap model or the mode will fall back to PrimaryOnly."
                .to_string(),
        );
    }

    // Policy mode with no rules
    if settings.routing_mode == RoutingMode::Policy && settings.policy_rules.is_empty() {
        warnings.push(
            "routing_mode is Policy but no rules defined; \
             will use default provider for all requests."
                .to_string(),
        );
    }

    // cheap_model references disabled provider
    if let Some(ref spec) = settings.cheap_model
        && let Some((slug, _)) = spec.split_once('/')
        && !settings.enabled.iter().any(|e| e == slug)
    {
        warnings.push(format!(
            "cheap_model '{}' references provider '{}' not in enabled list",
            spec, slug
        ));
    }

    warnings
}

// ──────────────────────────────────────────────────────────────────────
// Telemetry normalization (Phase 7)
// ──────────────────────────────────────────────────────────────────────

/// Canonical telemetry key format: `"{logical_role}|{provider_slug}|{model_id}"`.
///
/// Used by all routing decision telemetry to ensure consistent log indexing
/// across modes and providers.
pub fn canonical_telemetry_key(logical_role: &str, provider_slug: &str, model_id: &str) -> String {
    format!("{}|{}|{}", logical_role, provider_slug, model_id)
}

/// Enrich a `RouteDecision` telemetry key with resolved provider/model info.
///
/// Call this after the planner resolves a target to a concrete provider,
/// so the telemetry key reflects the actual provider and model used.
pub fn enrich_telemetry_key(decision: &mut RouteDecision, provider_slug: &str, model_id: &str) {
    // Extract logical role from the current key (the part before the first '|')
    let logical_role = decision
        .telemetry_key
        .split('|')
        .next()
        .unwrap_or(&decision.target);
    decision.telemetry_key = canonical_telemetry_key(logical_role, provider_slug, model_id);
}

/// Log a structured routing decision event for observability.
///
/// This emits a `tracing::info!` event with all decision fields in a
/// standardized format suitable for structured log aggregation.
pub fn log_routing_decision(decision: &RouteDecision, mode: &str) {
    tracing::info!(
        target = %decision.target,
        telemetry_key = %decision.telemetry_key,
        reason = %decision.reason,
        routing_mode = %mode,
        cascade = ?decision.cascade,
        advisor_active = decision.advisor.is_some(),
        advisor_ready = decision.advisor_ready,
        advisor_disabled_reason = ?decision.advisor_disabled_reason,
        executor_target = ?decision.executor_target,
        advisor_target = ?decision.advisor_target,
        tool_phase_synthesis = decision.tool_phase_synthesis,
        matched_rule = ?decision.matched_rule_index,
        fallback_count = decision.fallbacks.len(),
        candidate_count = decision.candidate_list.len(),
        rejection_count = decision.rejections.len(),
        diagnostics = ?decision.diagnostics,
        quality_score = decision.score.as_ref().map(|s| s.quality),
        cost_score = decision.score.as_ref().map(|s| s.cost),
        composite_score = decision.score.as_ref().map(|s| s.composite),
        "[route_planner] Routing decision"
    );
}

// ──────────────────────────────────────────────────────────────────────
// Health signal integration (Phase 8)
// ──────────────────────────────────────────────────────────────────────

/// Circuit breaker–aware health probe.
///
/// Reports provider health based on circuit breaker state:
/// - `Closed` (healthy) → 1.0
/// - `HalfOpen` (recovering) → 0.5
/// - `Open` (failing) → 0.0
///
/// When no circuit breaker data is available, returns a configurable default
/// (typically 0.8 to slightly penalize unknown providers vs known-healthy ones).
#[derive(Debug, Clone)]
pub struct CircuitBreakerHealthProbe {
    target: String,
    state: CircuitBreakerState,
}

/// Simplified circuit breaker state for routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitBreakerState {
    /// Provider is healthy — all requests succeed.
    Closed,
    /// Provider is recovering — limited requests sent.
    HalfOpen,
    /// Provider is failing — requests blocked.
    Open,
    /// No circuit breaker data available.
    Unknown,
}

impl CircuitBreakerHealthProbe {
    pub fn new(target: impl Into<String>, state: CircuitBreakerState) -> Self {
        Self {
            target: target.into(),
            state,
        }
    }
}

impl HealthProbe for CircuitBreakerHealthProbe {
    fn health_score(&self) -> f64 {
        match self.state {
            CircuitBreakerState::Closed => 1.0,
            CircuitBreakerState::HalfOpen => 0.5,
            CircuitBreakerState::Open => 0.0,
            CircuitBreakerState::Unknown => 0.8,
        }
    }

    fn probe_target(&self) -> &str {
        &self.target
    }
}

/// Build a health map from a collection of health probes.
///
/// Returns `target → health_score` suitable for `RoutePlannerInput::provider_health`.
pub fn build_health_map(probes: &[Box<dyn HealthProbe>]) -> HashMap<String, f64> {
    probes
        .iter()
        .map(|probe| (probe.probe_target().to_string(), probe.health_score()))
        .collect()
}

/// Latency-weighted health probe.
///
/// Combines circuit breaker state with latency data for a richer health signal.
/// High latency (>5s P50) degrades the health score even when the circuit is closed.
#[derive(Debug, Clone)]
pub struct LatencyWeightedHealthProbe {
    target: String,
    cb_state: CircuitBreakerState,
    latency_p50_ms: Option<f64>,
}

impl LatencyWeightedHealthProbe {
    pub fn new(
        target: impl Into<String>,
        cb_state: CircuitBreakerState,
        latency_p50_ms: Option<f64>,
    ) -> Self {
        Self {
            target: target.into(),
            cb_state,
            latency_p50_ms,
        }
    }
}

impl HealthProbe for LatencyWeightedHealthProbe {
    fn health_score(&self) -> f64 {
        let base = match self.cb_state {
            CircuitBreakerState::Closed => 1.0,
            CircuitBreakerState::HalfOpen => 0.5,
            CircuitBreakerState::Open => 0.0,
            CircuitBreakerState::Unknown => 0.8,
        };

        // Apply latency penalty when circuit is not open
        if base > 0.0 {
            if let Some(p50) = self.latency_p50_ms {
                // Degrade health for high latency:
                // 0-2000ms → no penalty
                // 2000-5000ms → gradual penalty (up to -0.3)
                // >5000ms → max penalty (-0.3)
                let penalty = if p50 > 5000.0 {
                    0.3
                } else if p50 > 2000.0 {
                    0.3 * (p50 - 2000.0) / 3000.0
                } else {
                    0.0
                };
                (base - penalty).max(0.1)
            } else {
                base
            }
        } else {
            base
        }
    }

    fn probe_target(&self) -> &str {
        &self.target
    }
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
