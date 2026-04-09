//! Smart LLM routing policy.
//!
//! Declarative rules that select a provider based on request context
//! (token count, vision, tools, budget). First matching rule wins.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A routing rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutingRule {
    /// Route to a specific provider if estimated tokens exceed threshold.
    LargeContext { threshold: u32, provider: String },
    /// Route to a specific provider if vision content detected.
    VisionContent { provider: String },
    /// Route to cheapest provider under a cost cap.
    CostOptimized { max_cost_per_m_usd: f64 },
    /// Use the lowest-latency provider.
    LowestLatency,
    /// Round-robin across providers.
    RoundRobin { providers: Vec<String> },
    /// Try primary, fall back on failure.
    Fallback {
        primary: String,
        fallbacks: Vec<String>,
    },
}

/// Context for a routing decision.
#[derive(Debug, Clone)]
pub struct RoutingContext {
    pub estimated_input_tokens: u32,
    pub has_vision: bool,
    pub has_tools: bool,
    pub requires_streaming: bool,
    pub budget_usd: Option<f64>,
}

/// A concrete route candidate that can be considered by policy rules.
#[derive(Debug, Clone)]
pub struct RouteCandidate {
    pub target: String,
    pub cost_per_m_usd: Option<f64>,
}

impl RouteCandidate {
    pub fn new(target: impl Into<String>, cost_per_m_usd: Option<f64>) -> Self {
        Self {
            target: target.into(),
            cost_per_m_usd,
        }
    }
}

/// Concrete routing decision produced by policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingDecision {
    pub target: String,
    pub fallbacks: Vec<String>,
    pub matched_rule_index: Option<usize>,
}

impl RoutingDecision {
    fn single(target: impl Into<String>, matched_rule_index: Option<usize>) -> Self {
        Self {
            target: target.into(),
            fallbacks: Vec::new(),
            matched_rule_index,
        }
    }

    fn with_fallbacks(
        target: impl Into<String>,
        fallbacks: Vec<String>,
        matched_rule_index: Option<usize>,
    ) -> Self {
        Self {
            target: target.into(),
            fallbacks,
            matched_rule_index,
        }
    }
}

/// Routing policy with ordered rules.
#[derive(Clone)]
pub struct RoutingPolicy {
    rules: Vec<RoutingRule>,
    default_provider: String,
    round_robin_counter: Arc<AtomicUsize>,
    /// Whether smart routing is enabled. When disabled, always uses default_provider.
    enabled: bool,
    /// Per-provider latency tracker for LowestLatency rule.
    latency_tracker: LatencyTracker,
}

/// Per-provider latency tracker using exponential moving average.
///
/// Call `record()` after each LLM response with the provider name and
/// latency. The `LowestLatency` routing rule consults this to pick the
/// provider with the lowest EMA latency.
#[derive(Debug, Clone, Default)]
pub struct LatencyTracker {
    /// provider → (ema_ms, sample_count)
    providers: HashMap<String, (f64, u64)>,
    /// EMA smoothing factor (0..1). Higher = more weight to recent samples.
    alpha: f64,
}

impl LatencyTracker {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            alpha: 0.3, // responsive to recent changes
        }
    }

    /// Record a latency sample for a provider.
    pub fn record(&mut self, provider: &str, latency_ms: f64) {
        let entry = self
            .providers
            .entry(provider.to_string())
            .or_insert((latency_ms, 0));
        entry.1 += 1;
        if entry.1 == 1 {
            // First sample: use raw value
            entry.0 = latency_ms;
        } else {
            // EMA update
            entry.0 = self.alpha * latency_ms + (1.0 - self.alpha) * entry.0;
        }
    }

    /// Get the provider with the lowest average latency.
    /// Returns None if no latency data recorded.
    pub fn get_fastest(&self) -> Option<String> {
        self.providers
            .iter()
            .min_by(|a, b| {
                a.1.0
                    .partial_cmp(&b.1.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(name, _)| name.clone())
    }

    /// Get the EMA latency for a specific provider.
    pub fn get_latency(&self, provider: &str) -> Option<f64> {
        self.providers.get(provider).map(|(ema, _)| *ema)
    }

    /// Number of providers with latency data.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

impl RoutingPolicy {
    pub fn new(default_provider: &str) -> Self {
        Self {
            rules: Vec::new(),
            default_provider: default_provider.to_string(),
            round_robin_counter: Arc::new(AtomicUsize::new(0)),
            enabled: true,
            latency_tracker: LatencyTracker::new(),
        }
    }

    /// Create from environment.
    pub fn from_env() -> Self {
        let default =
            std::env::var("LLM_DEFAULT_PROVIDER").unwrap_or_else(|_| "openai".to_string());
        let mut policy = Self::new(&default);

        if let (Ok(threshold), Ok(provider)) = (
            std::env::var("LLM_LARGE_CONTEXT_THRESHOLD"),
            std::env::var("LLM_LARGE_CONTEXT_PROVIDER"),
        ) && let Ok(t) = threshold.parse()
        {
            policy.add_rule(RoutingRule::LargeContext {
                threshold: t,
                provider,
            });
        }

        if let Ok(provider) = std::env::var("LLM_VISION_PROVIDER") {
            policy.add_rule(RoutingRule::VisionContent { provider });
        }

        policy
    }

    /// Add a rule.
    pub fn add_rule(&mut self, rule: RoutingRule) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// Select a provider for the given context.
    ///
    /// If smart routing is disabled, always returns the default provider.
    pub fn select_provider(&self, ctx: &RoutingContext) -> String {
        self.select_decision(ctx, &[]).target
    }

    /// Select a concrete routing decision for the given context.
    ///
    /// `available_targets` is used by cost-aware rules to pick an actual
    /// route instead of falling back to a placeholder alias.
    pub fn select_decision(
        &self,
        ctx: &RoutingContext,
        available_targets: &[RouteCandidate],
    ) -> RoutingDecision {
        if !self.enabled {
            return RoutingDecision::single(self.default_provider.clone(), None);
        }
        for (index, rule) in self.rules.iter().enumerate() {
            if let Some(decision) = self.matches_rule(rule, ctx, available_targets, Some(index)) {
                return decision;
            }
        }
        RoutingDecision::single(self.default_provider.clone(), None)
    }

    fn matches_rule(
        &self,
        rule: &RoutingRule,
        ctx: &RoutingContext,
        available_targets: &[RouteCandidate],
        matched_rule_index: Option<usize>,
    ) -> Option<RoutingDecision> {
        match rule {
            RoutingRule::LargeContext {
                threshold,
                provider,
            } => {
                if ctx.estimated_input_tokens > *threshold {
                    Some(RoutingDecision::single(
                        provider.clone(),
                        matched_rule_index,
                    ))
                } else {
                    None
                }
            }
            RoutingRule::VisionContent { provider } => {
                if ctx.has_vision {
                    Some(RoutingDecision::single(
                        provider.clone(),
                        matched_rule_index,
                    ))
                } else {
                    None
                }
            }
            RoutingRule::CostOptimized { max_cost_per_m_usd } => available_targets
                .iter()
                .filter_map(|candidate| {
                    candidate
                        .cost_per_m_usd
                        .map(|cost| (candidate.target.clone(), cost))
                })
                .filter(|(_, cost)| *cost <= *max_cost_per_m_usd)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(target, _)| RoutingDecision::single(target, matched_rule_index)),
            RoutingRule::LowestLatency => self
                .latency_tracker
                .get_fastest()
                .map(|target| RoutingDecision::single(target, matched_rule_index)),
            RoutingRule::RoundRobin { providers } => {
                if providers.is_empty() {
                    return None;
                }
                let idx = self.round_robin_counter.fetch_add(1, Ordering::Relaxed);
                Some(RoutingDecision::single(
                    providers[idx % providers.len()].clone(),
                    matched_rule_index,
                ))
            }
            RoutingRule::Fallback { primary, fallbacks } => Some(RoutingDecision::with_fallbacks(
                primary.clone(),
                fallbacks.clone(),
                matched_rule_index,
            )),
        }
    }

    pub fn matches_rule_for_summary(
        &self,
        rule: &RoutingRule,
        ctx: &RoutingContext,
        available_targets: &[RouteCandidate],
    ) -> Option<RoutingDecision> {
        self.matches_rule(rule, ctx, available_targets, None)
    }

    /// Number of rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Default provider name.
    pub fn default_provider(&self) -> &str {
        &self.default_provider
    }

    /// Whether smart routing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable smart routing.
    ///
    /// When disabled, `select_provider()` always returns the default provider,
    /// ignoring all rules. This is the "Smart Routing" toggle in Scrappy UI.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Record a latency sample for a provider.
    ///
    /// Call this after each LLM response with the provider name and
    /// response time in milliseconds. The `LowestLatency` rule uses
    /// this data to route to the fastest provider.
    pub fn record_latency(&mut self, provider: &str, latency_ms: f64) {
        self.latency_tracker.record(provider, latency_ms);
    }

    /// Get the latency tracker (read-only) for inspection.
    pub fn latency_tracker(&self) -> &LatencyTracker {
        &self.latency_tracker
    }

    /// Get all rules (read-only, for UI display).
    pub fn rules(&self) -> &[RoutingRule] {
        &self.rules
    }

    /// Remove a rule by index.
    ///
    /// Returns `Err` if the index is out of bounds.
    pub fn remove_rule(&mut self, index: usize) -> Result<RoutingRule, String> {
        if index >= self.rules.len() {
            return Err(format!(
                "Rule index {} out of bounds (have {} rules)",
                index,
                self.rules.len()
            ));
        }
        Ok(self.rules.remove(index))
    }

    /// Reorder a rule from one position to another.
    ///
    /// Moves the rule at `from` to `to`, shifting other rules accordingly.
    /// Returns `Err` if either index is out of bounds.
    pub fn reorder_rules(&mut self, from: usize, to: usize) -> Result<(), String> {
        let len = self.rules.len();
        if from >= len {
            return Err(format!(
                "Source index {} out of bounds (have {} rules)",
                from, len
            ));
        }
        if to >= len {
            return Err(format!(
                "Target index {} out of bounds (have {} rules)",
                to, len
            ));
        }
        if from == to {
            return Ok(());
        }
        let rule = self.rules.remove(from);
        self.rules.insert(to, rule);
        Ok(())
    }

    /// Set the default provider.
    pub fn set_default_provider(&mut self, provider: impl Into<String>) {
        self.default_provider = provider.into();
    }
}

/// Serializable summary of a routing rule for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRuleSummary {
    pub index: usize,
    pub rule_type: String,
    pub description: String,
    pub rule: RoutingRule,
}

impl RoutingRuleSummary {
    /// Build summaries from a policy's rules.
    pub fn from_policy(policy: &RoutingPolicy) -> Vec<Self> {
        policy
            .rules()
            .iter()
            .enumerate()
            .map(|(i, rule)| {
                let (rule_type, description) = match rule {
                    RoutingRule::LargeContext {
                        threshold,
                        provider,
                    } => (
                        "large_context".into(),
                        format!("If tokens > {}, use {}", threshold, provider),
                    ),
                    RoutingRule::VisionContent { provider } => (
                        "vision".into(),
                        format!("If vision content, use {}", provider),
                    ),
                    RoutingRule::CostOptimized { max_cost_per_m_usd } => (
                        "cost_optimized".into(),
                        format!(
                            "Route to the cheapest available target at or under ${}/M tokens",
                            max_cost_per_m_usd
                        ),
                    ),
                    RoutingRule::LowestLatency => (
                        "lowest_latency".into(),
                        "Route to provider with lowest average latency".into(),
                    ),
                    RoutingRule::RoundRobin { providers } => (
                        "round_robin".into(),
                        format!("Round-robin across: {}", providers.join(", ")),
                    ),
                    RoutingRule::Fallback { primary, fallbacks } => (
                        "fallback".into(),
                        format!("Try {}, fallback to {}", primary, fallbacks.join(", ")),
                    ),
                };
                Self {
                    index: i,
                    rule_type,
                    description,
                    rule: rule.clone(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_ctx() -> RoutingContext {
        RoutingContext {
            estimated_input_tokens: 1000,
            has_vision: false,
            has_tools: false,
            requires_streaming: false,
            budget_usd: None,
        }
    }

    #[test]
    fn test_default_provider() {
        let policy = RoutingPolicy::new("openai");
        assert_eq!(policy.select_provider(&base_ctx()), "openai");
    }

    #[test]
    fn test_large_context_rule() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LargeContext {
            threshold: 100_000,
            provider: "gemini".into(),
        });
        let mut ctx = base_ctx();
        ctx.estimated_input_tokens = 200_000;
        assert_eq!(policy.select_provider(&ctx), "gemini");
    }

    #[test]
    fn test_vision_rule() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini-vision".into(),
        });
        let mut ctx = base_ctx();
        ctx.has_vision = true;
        assert_eq!(policy.select_provider(&ctx), "gemini-vision");
    }

    #[test]
    fn test_cost_optimized() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::CostOptimized {
            max_cost_per_m_usd: 5.0,
        });
        let decision = policy.select_decision(
            &base_ctx(),
            &[
                RouteCandidate::new("primary", Some(12.0)),
                RouteCandidate::new("cheap", Some(2.0)),
            ],
        );
        assert_eq!(decision.target, "cheap");
    }

    #[test]
    fn test_round_robin() {
        let mut policy = RoutingPolicy::new("default");
        policy.add_rule(RoutingRule::RoundRobin {
            providers: vec!["a".into(), "b".into(), "c".into()],
        });
        let ctx = base_ctx();
        let p1 = policy.select_provider(&ctx);
        let p2 = policy.select_provider(&ctx);
        let p3 = policy.select_provider(&ctx);
        assert_eq!(p1, "a");
        assert_eq!(p2, "b");
        assert_eq!(p3, "c");
    }

    #[test]
    fn test_fallback_returns_primary() {
        let mut policy = RoutingPolicy::new("default");
        policy.add_rule(RoutingRule::Fallback {
            primary: "main".into(),
            fallbacks: vec!["backup".into()],
        });
        let decision = policy.select_decision(&base_ctx(), &[]);
        assert_eq!(decision.target, "main");
        assert_eq!(decision.fallbacks, vec!["backup"]);
    }

    #[test]
    fn test_rule_priority_first_wins() {
        let mut policy = RoutingPolicy::new("default");
        policy.add_rule(RoutingRule::VisionContent {
            provider: "first".into(),
        });
        policy.add_rule(RoutingRule::VisionContent {
            provider: "second".into(),
        });
        let mut ctx = base_ctx();
        ctx.has_vision = true;
        assert_eq!(policy.select_provider(&ctx), "first");
    }

    #[test]
    fn test_from_env() {
        // Without env vars set, defaults to "openai"
        let policy = RoutingPolicy::from_env();
        assert!(!policy.default_provider().is_empty());
    }

    #[test]
    fn test_smart_routing_toggle() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });
        let mut ctx = base_ctx();
        ctx.has_vision = true;

        // Enabled: rule fires
        assert!(policy.is_enabled());
        assert_eq!(policy.select_provider(&ctx), "gemini");

        // Disabled: falls back to default
        policy.set_enabled(false);
        assert!(!policy.is_enabled());
        assert_eq!(policy.select_provider(&ctx), "openai");

        // Re-enabled: rule fires again
        policy.set_enabled(true);
        assert_eq!(policy.select_provider(&ctx), "gemini");
    }

    #[test]
    fn test_latency_tracker_basic() {
        let mut tracker = LatencyTracker::new();
        tracker.record("openai", 200.0);
        tracker.record("gemini", 100.0);
        assert_eq!(tracker.get_fastest().as_deref(), Some("gemini"));
        assert_eq!(tracker.provider_count(), 2);
    }

    #[test]
    fn test_latency_tracker_ema() {
        let mut tracker = LatencyTracker::new();
        tracker.record("p", 1000.0); // first sample
        tracker.record("p", 100.0); // EMA: 0.3*100 + 0.7*1000 = 730
        let latency = tracker.get_latency("p").unwrap();
        assert!((latency - 730.0).abs() < 1.0);
    }

    #[test]
    fn test_latency_tracker_empty() {
        let tracker = LatencyTracker::new();
        assert!(tracker.get_fastest().is_none());
    }

    #[test]
    fn test_lowest_latency_rule() {
        let mut policy = RoutingPolicy::new("default");
        policy.add_rule(RoutingRule::LowestLatency);
        policy.record_latency("openai", 300.0);
        policy.record_latency("gemini", 150.0);
        policy.record_latency("anthropic", 200.0);

        let selected = policy.select_provider(&base_ctx());
        assert_eq!(selected, "gemini");
    }

    #[test]
    fn test_lowest_latency_no_data_falls_through() {
        let mut policy = RoutingPolicy::new("default");
        policy.add_rule(RoutingRule::LowestLatency);
        // No latency data recorded
        assert_eq!(policy.select_provider(&base_ctx()), "default");
    }

    // ── CRUD method tests ─────────────────────────────────────────────

    #[test]
    fn test_rules_accessor() {
        let mut policy = RoutingPolicy::new("openai");
        assert!(policy.rules().is_empty());
        policy.add_rule(RoutingRule::LowestLatency);
        assert_eq!(policy.rules().len(), 1);
    }

    #[test]
    fn test_remove_rule() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LowestLatency);
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });
        let removed = policy.remove_rule(0).unwrap();
        assert!(matches!(removed, RoutingRule::LowestLatency));
        assert_eq!(policy.rule_count(), 1);
    }

    #[test]
    fn test_remove_rule_out_of_bounds() {
        let mut policy = RoutingPolicy::new("openai");
        assert!(policy.remove_rule(0).is_err());
    }

    #[test]
    fn test_reorder_rules() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LowestLatency);
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });
        policy.add_rule(RoutingRule::LargeContext {
            threshold: 100_000,
            provider: "claude".into(),
        });

        // Move last to first
        policy.reorder_rules(2, 0).unwrap();
        assert!(matches!(
            policy.rules()[0],
            RoutingRule::LargeContext { .. }
        ));
        assert!(matches!(policy.rules()[1], RoutingRule::LowestLatency));
        assert!(matches!(
            policy.rules()[2],
            RoutingRule::VisionContent { .. }
        ));
    }

    #[test]
    fn test_reorder_rules_same_index() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LowestLatency);
        assert!(policy.reorder_rules(0, 0).is_ok());
    }

    #[test]
    fn test_reorder_rules_out_of_bounds() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LowestLatency);
        assert!(policy.reorder_rules(0, 5).is_err());
        assert!(policy.reorder_rules(5, 0).is_err());
    }

    #[test]
    fn test_set_default_provider() {
        let mut policy = RoutingPolicy::new("openai");
        assert_eq!(policy.default_provider(), "openai");
        policy.set_default_provider("anthropic");
        assert_eq!(policy.default_provider(), "anthropic");

        // Verify it affects routing when disabled
        policy.set_enabled(false);
        assert_eq!(policy.select_provider(&base_ctx()), "anthropic");
    }

    #[test]
    fn test_routing_rule_summary() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });
        policy.add_rule(RoutingRule::RoundRobin {
            providers: vec!["a".into(), "b".into()],
        });
        policy.add_rule(RoutingRule::Fallback {
            primary: "main".into(),
            fallbacks: vec!["backup".into()],
        });

        let summaries = RoutingRuleSummary::from_policy(&policy);
        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].index, 0);
        assert_eq!(summaries[0].rule_type, "vision");
        assert!(summaries[0].description.contains("gemini"));
        assert_eq!(summaries[1].rule_type, "round_robin");
        assert!(summaries[1].description.contains("a, b"));
        assert_eq!(summaries[2].rule_type, "fallback");
        assert!(summaries[2].description.contains("main"));
    }

    #[test]
    fn test_routing_rule_summary_serializable() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LowestLatency);
        let summaries = RoutingRuleSummary::from_policy(&policy);
        let json = serde_json::to_string(&summaries).unwrap();
        assert!(json.contains("lowest_latency"));
        let deser: Vec<RoutingRuleSummary> = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.len(), 1);
    }
}
