//! Smart LLM routing policy.
//!
//! Declarative rules that select a provider based on request context
//! (token count, vision, tools, budget). First matching rule wins.

use serde::{Deserialize, Serialize};
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
pub struct RoutingContext {
    pub estimated_input_tokens: u32,
    pub has_vision: bool,
    pub has_tools: bool,
    pub requires_streaming: bool,
    pub budget_usd: Option<f64>,
}

/// Routing policy with ordered rules.
pub struct RoutingPolicy {
    rules: Vec<RoutingRule>,
    default_provider: String,
    round_robin_counter: Arc<AtomicUsize>,
}

impl RoutingPolicy {
    pub fn new(default_provider: &str) -> Self {
        Self {
            rules: Vec::new(),
            default_provider: default_provider.to_string(),
            round_robin_counter: Arc::new(AtomicUsize::new(0)),
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
        ) {
            if let Ok(t) = threshold.parse() {
                policy.add_rule(RoutingRule::LargeContext {
                    threshold: t,
                    provider,
                });
            }
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
    pub fn select_provider(&self, ctx: &RoutingContext) -> String {
        for rule in &self.rules {
            if let Some(provider) = self.matches_rule(rule, ctx) {
                return provider;
            }
        }
        self.default_provider.clone()
    }

    fn matches_rule(&self, rule: &RoutingRule, ctx: &RoutingContext) -> Option<String> {
        match rule {
            RoutingRule::LargeContext {
                threshold,
                provider,
            } => {
                if ctx.estimated_input_tokens > *threshold {
                    Some(provider.clone())
                } else {
                    None
                }
            }
            RoutingRule::VisionContent { provider } => {
                if ctx.has_vision {
                    Some(provider.clone())
                } else {
                    None
                }
            }
            RoutingRule::CostOptimized { max_cost_per_m_usd } => {
                if let Some(budget) = ctx.budget_usd {
                    if budget <= *max_cost_per_m_usd {
                        Some(self.default_provider.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            RoutingRule::LowestLatency => {
                // In real impl, would check latency metrics
                None
            }
            RoutingRule::RoundRobin { providers } => {
                if providers.is_empty() {
                    return None;
                }
                let idx = self.round_robin_counter.fetch_add(1, Ordering::Relaxed);
                Some(providers[idx % providers.len()].clone())
            }
            RoutingRule::Fallback { primary, .. } => Some(primary.clone()),
        }
    }

    /// Number of rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Default provider name.
    pub fn default_provider(&self) -> &str {
        &self.default_provider
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
        let mut ctx = base_ctx();
        ctx.budget_usd = Some(3.0);
        assert_eq!(policy.select_provider(&ctx), "openai");
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
        assert_eq!(policy.select_provider(&base_ctx()), "main");
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
}
