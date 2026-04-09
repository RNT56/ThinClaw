//! Unified route planner for all routing modes.
//!
//! Replaces the dual-path routing logic (SmartRoutingProvider + RoutingPolicy)
//! with a single `RoutePlanner::plan()` call that supports four strategies:
//!
//! - **PrimaryOnly** — all requests → primary model
//! - **CheapSplit** — classify by complexity, preserve cascade escalation
//! - **AdvisorExecutor** — executor runs everything, consults advisor on demand
//! - **Policy** — delegated to `RoutingPolicy` rule engine
//!
//! Reference: <https://claude.com/blog/the-advisor-strategy>

use std::collections::HashMap;

use crate::llm::routing_policy::{RouteCandidate, RoutingContext, RoutingPolicy};
use crate::llm::smart_routing::{SmartRoutingConfig, TaskComplexity, classify_message};
use crate::settings::RoutingMode;

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
#[derive(Debug, Clone)]
pub struct ProviderCapabilities {
    pub supports_streaming: bool,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_thinking: bool,
    pub max_context_tokens: Option<u32>,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            supports_streaming: true,
            supports_tools: true,
            supports_vision: true,
            supports_thinking: false,
            max_context_tokens: None,
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
    /// Canonical telemetry key: "logical_role|provider_slug|model_id".
    pub telemetry_key: String,
    /// Index of the matched policy rule, if any.
    pub matched_rule_index: Option<usize>,
    /// Post-response cascade behavior.
    pub cascade: CascadePolicy,
    /// Advisor configuration (AdvisorExecutor mode only).
    pub advisor: Option<AdvisorConfig>,
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
            telemetry_key: "primary||".to_string(),
            matched_rule_index: None,
            cascade: CascadePolicy::None,
            advisor: None,
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

impl RouteScorer {
    pub fn new(weights: RoutingWeights) -> Self {
        Self { weights }
    }

    /// Score a candidate. Returns `None` if the candidate fails a hard gate.
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
    ) -> Option<RoutingScoreBreakdown> {
        // ── Hard gates ─────────────────────────────────────────
        if required.streaming && !capabilities.supports_streaming {
            return None;
        }
        if required.tool_use && !capabilities.supports_tools {
            return None;
        }
        if required.vision && !capabilities.supports_vision {
            return None;
        }
        if required.extended_thinking && !capabilities.supports_thinking {
            return None;
        }

        // Context window gate
        if let Some(max_ctx) = capabilities.max_context_tokens {
            if estimated_input_tokens > max_ctx {
                return None;
            }
        }

        // ── Dimension scores ───────────────────────────────────
        let quality = model_quality_tier(&candidate.target);
        let cost = cost_score(candidate.cost_per_m_usd);
        let latency = latency_score(latency_p50_ms);

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

        Some(RoutingScoreBreakdown {
            quality,
            cost,
            latency,
            health,
            policy_bias,
            composite,
        })
    }
}

/// Quality tier lookup by model target.
fn model_quality_tier(target: &str) -> f64 {
    let lower = target.to_lowercase();
    // Check model family patterns
    if lower.contains("opus") {
        0.95
    } else if lower.contains("gpt-4o") && !lower.contains("mini") {
        0.90
    } else if lower.contains("sonnet") {
        0.85
    } else if lower.contains("gemini-2") && lower.contains("pro") {
        0.85
    } else if lower.contains("flash") && !lower.contains("lite") {
        0.60
    } else if lower.contains("haiku") {
        0.50
    } else if lower.contains("gpt-4o-mini") || lower.contains("4o-mini") {
        0.50
    } else if lower.contains("flash-lite") {
        0.35
    } else if lower == "primary" {
        0.85 // assume primary is a quality model
    } else if lower == "cheap" {
        0.50 // assume cheap is a cost model
    } else {
        0.50 // unknown
    }
}

/// Cost score: cheaper is higher (inverted, normalized 0–1).
fn cost_score(cost_per_m_usd: Option<f64>) -> f64 {
    match cost_per_m_usd {
        Some(c) if c > 0.0 => {
            // Normalize: $0/M → 1.0, $100/M → ~0.0
            1.0 / (1.0 + c / 10.0)
        }
        Some(_) => 1.0,  // free or zero cost
        None => 0.5,     // unknown cost
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
        None => 0.5,     // unknown latency
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
    pub fn plan(
        &self,
        input: &RoutePlannerInput,
        policy: Option<&RoutingPolicy>,
    ) -> RouteDecision {
        // ── 1. Explicit override ───────────────────────────────
        if let Some(ref model_override) = input.model_override {
            return RouteDecision {
                target: model_override.clone(),
                fallbacks: Vec::new(),
                reason: format!("Explicit model override: {}", model_override),
                score: None,
                telemetry_key: format!("override||{}", model_override),
                matched_rule_index: None,
                cascade: CascadePolicy::None,
                advisor: None,
                tool_phase_synthesis: false,
            };
        }

        // ── 2. Mode-specific logic ─────────────────────────────
        match input.routing_mode {
            RoutingMode::PrimaryOnly => self.plan_primary_only(),
            RoutingMode::CheapSplit => self.plan_cheap_split(input),
            RoutingMode::AdvisorExecutor => self.plan_advisor_executor(input),
            RoutingMode::Policy => self.plan_policy(input, policy),
        }
    }

    // -- PrimaryOnly --

    fn plan_primary_only(&self) -> RouteDecision {
        RouteDecision::primary("PrimaryOnly mode")
    }

    // -- CheapSplit (preserved) --

    fn plan_cheap_split(&self, input: &RoutePlannerInput) -> RouteDecision {
        // Hard override: tools/streaming → always primary
        if input.required_capabilities.tool_use || input.required_capabilities.streaming {
            let mut decision = RouteDecision::primary(
                "CheapSplit: tool/streaming request → primary (always)"
            );
            decision.telemetry_key = "primary||".to_string();

            // Tool-phase synthesis decision
            if input.required_capabilities.tool_use
                && self.tool_phase_synthesis_enabled
                && input.model_override.is_none()
                && self.has_cheap_candidate(input)
            {
                decision.tool_phase_synthesis = true;
            }

            return decision;
        }

        // Classify by message content
        let msg = input.last_user_message.as_deref().unwrap_or("");
        let complexity = classify_message(msg, &self.cheap_split_config);

        match complexity {
            TaskComplexity::Simple => RouteDecision {
                target: "cheap".to_string(),
                fallbacks: vec!["primary".to_string()],
                reason: "CheapSplit: Simple task → cheap model".to_string(),
                score: None,
                telemetry_key: "cheap||".to_string(),
                matched_rule_index: None,
                cascade: CascadePolicy::None,
                advisor: None,
                tool_phase_synthesis: false,
            },
            TaskComplexity::Complex => RouteDecision {
                target: "primary".to_string(),
                fallbacks: Vec::new(),
                reason: "CheapSplit: Complex task → primary model".to_string(),
                score: None,
                telemetry_key: "primary||".to_string(),
                matched_rule_index: None,
                cascade: CascadePolicy::None,
                advisor: None,
                tool_phase_synthesis: false,
            },
            TaskComplexity::Moderate => {
                let cascade = if self.cascade_enabled {
                    CascadePolicy::InspectAndEscalate
                } else {
                    CascadePolicy::None
                };
                let reason = if self.cascade_enabled {
                    "CheapSplit: Moderate task → cheap (cascade enabled)"
                } else {
                    "CheapSplit: Moderate task → cheap (cascade disabled)"
                };
                RouteDecision {
                    target: "cheap".to_string(),
                    fallbacks: vec!["primary".to_string()],
                    reason: reason.to_string(),
                    score: None,
                    telemetry_key: "cheap||".to_string(),
                    matched_rule_index: None,
                    cascade,
                    advisor: None,
                    tool_phase_synthesis: false,
                }
            }
        }
    }

    // -- AdvisorExecutor (new) --

    fn plan_advisor_executor(&self, input: &RoutePlannerInput) -> RouteDecision {
        // Everything goes to executor (cheap model slot), including tools and streaming.
        // Advisor config is attached so the dispatcher can inject the consult_advisor tool.
        let has_cheap = self.has_cheap_candidate(input);

        let (target, reason) = if has_cheap {
            (
                "cheap".to_string(),
                "AdvisorExecutor: executor (cheap) handles all requests".to_string(),
            )
        } else {
            // No cheap model configured — fall back to primary-only.
            // This gracefully degrades: user gets the advisor prompt but no cost savings.
            (
                "primary".to_string(),
                "AdvisorExecutor: no executor model configured, using primary".to_string(),
            )
        };

        let advisor = if has_cheap {
            Some(AdvisorConfig {
                advisor_target: "primary".to_string(),
                max_advisor_calls: self.advisor_max_calls,
                advisor_system_prompt: ADVISOR_SYSTEM_PROMPT.to_string(),
            })
        } else {
            None
        };

        RouteDecision {
            target,
            fallbacks: vec!["primary".to_string()],
            reason,
            score: None,
            telemetry_key: if has_cheap {
                "executor||".to_string()
            } else {
                "primary||".to_string()
            },
            matched_rule_index: None,
            cascade: CascadePolicy::None,
            advisor,
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
        let reason = decision
            .matched_rule_index
            .map(|idx| format!("Policy rule {} matched", idx))
            .unwrap_or_else(|| "Policy default target".to_string());

        RouteDecision {
            target: decision.target,
            fallbacks: decision.fallbacks,
            reason,
            score: None,
            telemetry_key: decision
                .matched_rule_index
                .map(|idx| format!("policy_rule_{}||", idx))
                .unwrap_or_else(|| "policy_default||".to_string()),
            matched_rule_index: decision.matched_rule_index,
            cascade: CascadePolicy::None,
            advisor: None,
            tool_phase_synthesis: false,
        }
    }

    // -- Helper --

    fn has_cheap_candidate(&self, input: &RoutePlannerInput) -> bool {
        input
            .candidates
            .iter()
            .any(|c| c.target == "cheap")
    }
}

// ──────────────────────────────────────────────────────────────────────
// Config validation (Phase 7)
// ──────────────────────────────────────────────────────────────────────

/// Validate provider settings and return warnings.
pub fn validate_providers_settings(
    settings: &crate::settings::ProvidersSettings,
) -> Vec<String> {
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
    if let Some(ref spec) = settings.cheap_model {
        if let Some((slug, _)) = spec.split_once('/') {
            if !settings.enabled.iter().any(|e| e == slug) {
                warnings.push(format!(
                    "cheap_model '{}' references provider '{}' not in enabled list",
                    spec, slug
                ));
            }
        }
    }

    warnings
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::routing_policy::RouteCandidate;

    fn default_input() -> RoutePlannerInput {
        RoutePlannerInput {
            required_capabilities: RequiredCapabilities::default(),
            routing_mode: RoutingMode::PrimaryOnly,
            routing_context: RoutingContext {
                estimated_input_tokens: 100,
                has_vision: false,
                has_tools: false,
                requires_streaming: false,
                budget_usd: None,
            },
            model_override: None,
            provider_health: HashMap::new(),
            candidates: vec![
                RouteCandidate::new("primary", Some(30.0)),
                RouteCandidate::new("cheap", Some(1.0)),
            ],
            turn_cost_usd: 0.0,
            budget_utilization: None,
            last_user_message: None,
        }
    }

    fn planner() -> RoutePlanner {
        RoutePlanner::new(true, false, 3)
    }

    // -- Override precedence --

    #[test]
    fn override_takes_precedence_over_mode() {
        let p = planner();
        let mut input = default_input();
        input.model_override = Some("openai/gpt-4o".to_string());
        input.routing_mode = RoutingMode::CheapSplit;
        let d = p.plan(&input, None);
        assert_eq!(d.target, "openai/gpt-4o");
        assert!(d.reason.contains("override"));
    }

    // -- PrimaryOnly --

    #[test]
    fn primary_only_always_primary() {
        let p = planner();
        let input = default_input();
        let d = p.plan(&input, None);
        assert_eq!(d.target, "primary");
    }

    // -- CheapSplit --

    #[test]
    fn cheap_split_simple_goes_cheap() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::CheapSplit;
        input.last_user_message = Some("hello".to_string());
        let d = p.plan(&input, None);
        assert_eq!(d.target, "cheap");
    }

    #[test]
    fn cheap_split_complex_goes_primary() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::CheapSplit;
        input.last_user_message = Some("implement a new caching layer".to_string());
        let d = p.plan(&input, None);
        assert_eq!(d.target, "primary");
    }

    #[test]
    fn cheap_split_moderate_with_cascade() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::CheapSplit;
        // A message that's moderate: not matching simple or complex keywords, mid-length
        input.last_user_message = Some(
            "Can you tell me about the differences between these approaches?"
                .to_string(),
        );
        let d = p.plan(&input, None);
        assert_eq!(d.target, "cheap");
        assert_eq!(d.cascade, CascadePolicy::InspectAndEscalate);
    }

    #[test]
    fn cheap_split_tools_always_primary() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::CheapSplit;
        input.required_capabilities.tool_use = true;
        input.last_user_message = Some("hello".to_string());
        let d = p.plan(&input, None);
        assert_eq!(d.target, "primary");
    }

    #[test]
    fn cheap_split_streaming_always_primary() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::CheapSplit;
        input.required_capabilities.streaming = true;
        input.last_user_message = Some("hello".to_string());
        let d = p.plan(&input, None);
        assert_eq!(d.target, "primary");
    }

    #[test]
    fn cheap_split_tool_phase_synthesis() {
        let p = RoutePlanner::new(true, true, 3);
        let mut input = default_input();
        input.routing_mode = RoutingMode::CheapSplit;
        input.required_capabilities.tool_use = true;
        let d = p.plan(&input, None);
        assert_eq!(d.target, "primary");
        assert!(d.tool_phase_synthesis);
    }

    // -- AdvisorExecutor --

    #[test]
    fn advisor_executor_routes_to_executor() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::AdvisorExecutor;
        let d = p.plan(&input, None);
        assert_eq!(d.target, "cheap");
        assert!(d.advisor.is_some());
    }

    #[test]
    fn advisor_executor_tools_go_to_executor() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::AdvisorExecutor;
        input.required_capabilities.tool_use = true;
        let d = p.plan(&input, None);
        // In AdvisorExecutor, tools go to executor (cheap), NOT primary
        assert_eq!(d.target, "cheap");
        assert!(d.advisor.is_some());
    }

    #[test]
    fn advisor_executor_streaming_goes_to_executor() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::AdvisorExecutor;
        input.required_capabilities.streaming = true;
        let d = p.plan(&input, None);
        assert_eq!(d.target, "cheap");
    }

    #[test]
    fn advisor_executor_no_cheap_falls_back() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::AdvisorExecutor;
        input.candidates = vec![RouteCandidate::new("primary", Some(30.0))];
        let d = p.plan(&input, None);
        assert_eq!(d.target, "primary");
        assert!(d.advisor.is_none());
    }

    #[test]
    fn advisor_config_max_calls() {
        let p = RoutePlanner::new(true, false, 5);
        let mut input = default_input();
        input.routing_mode = RoutingMode::AdvisorExecutor;
        let d = p.plan(&input, None);
        assert_eq!(d.advisor.as_ref().unwrap().max_advisor_calls, 5);
    }

    // -- Policy --

    #[test]
    fn policy_delegates_to_policy_engine() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::Policy;
        let policy = RoutingPolicy::new("primary");
        let d = p.plan(&input, Some(&policy));
        // Default policy returns default_provider = "primary"
        assert_eq!(d.target, "primary");
    }

    #[test]
    fn policy_without_policy_falls_back() {
        let p = planner();
        let mut input = default_input();
        input.routing_mode = RoutingMode::Policy;
        let d = p.plan(&input, None);
        assert_eq!(d.target, "primary");
        assert!(d.reason.contains("no policy"));
    }

    // -- Scorer --

    #[test]
    fn scorer_hard_gate_streaming() {
        let scorer = RouteScorer::new(RoutingWeights::default());
        let caps = ProviderCapabilities {
            supports_streaming: false,
            ..Default::default()
        };
        let required = RequiredCapabilities {
            streaming: true,
            ..Default::default()
        };
        let result = scorer.score(
            &RouteCandidate::new("test", Some(10.0)),
            &caps,
            &required,
            1.0,
            None,
            0.0,
            None,
            100,
        );
        assert!(result.is_none());
    }

    #[test]
    fn scorer_hard_gate_context_window() {
        let scorer = RouteScorer::new(RoutingWeights::default());
        let caps = ProviderCapabilities {
            max_context_tokens: Some(4096),
            ..Default::default()
        };
        let required = RequiredCapabilities::default();
        let result = scorer.score(
            &RouteCandidate::new("test", Some(10.0)),
            &caps,
            &required,
            1.0,
            None,
            0.0,
            None,
            8000, // exceeds context window
        );
        assert!(result.is_none());
    }

    #[test]
    fn scorer_budget_pressure_high() {
        let scorer = RouteScorer::new(RoutingWeights::default());
        let caps = ProviderCapabilities::default();
        let required = RequiredCapabilities::default();

        let normal = scorer
            .score(
                &RouteCandidate::new("test", Some(10.0)),
                &caps,
                &required,
                1.0,
                None,
                0.0,
                Some(0.3), // low budget usage
                100,
            )
            .unwrap();

        let high_pressure = scorer
            .score(
                &RouteCandidate::new("test", Some(10.0)),
                &caps,
                &required,
                1.0,
                None,
                0.0,
                Some(0.95), // near budget limit
                100,
            )
            .unwrap();

        // High budget pressure should increase cost weight, changing composite
        assert!(high_pressure.composite != normal.composite);
    }

    // -- Quality tiers --

    #[test]
    fn quality_tier_known_models() {
        assert!(model_quality_tier("opus") > 0.9);
        assert!(model_quality_tier("sonnet") > 0.8);
        assert!(model_quality_tier("haiku") < 0.6);
        assert!(model_quality_tier("primary") > 0.8);
        assert!(model_quality_tier("cheap") < 0.6);
    }

    // -- Config validation --

    #[test]
    fn validate_advisor_without_cheap_model() {
        let mut settings = crate::settings::ProvidersSettings::default();
        settings.routing_mode = RoutingMode::AdvisorExecutor;
        let warnings = validate_providers_settings(&settings);
        assert!(warnings.iter().any(|w| w.contains("AdvisorExecutor")));
    }

    #[test]
    fn validate_policy_without_rules() {
        let mut settings = crate::settings::ProvidersSettings::default();
        settings.routing_mode = RoutingMode::Policy;
        let warnings = validate_providers_settings(&settings);
        assert!(warnings.iter().any(|w| w.contains("no rules")));
    }

    // -- Serde roundtrip --

    #[test]
    fn routing_mode_serde_roundtrip() {
        // Existing values
        let json = serde_json::to_string(&RoutingMode::CheapSplit).unwrap();
        assert_eq!(json, "\"cheap_split\"");
        let back: RoutingMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, RoutingMode::CheapSplit);

        // New value
        let json = serde_json::to_string(&RoutingMode::AdvisorExecutor).unwrap();
        assert_eq!(json, "\"advisor_executor\"");
        let back: RoutingMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, RoutingMode::AdvisorExecutor);

        // Alias
        let back: RoutingMode = serde_json::from_str("\"advisor\"").unwrap();
        assert_eq!(back, RoutingMode::AdvisorExecutor);
    }
}
