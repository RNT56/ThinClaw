//! Route simulation and advisor diagnostics: read-only "what would the planner
//! pick" entry points used by the CLI/gateway and advisor-readiness probes.
//! These run the planner without issuing a request.

use crate::llm::route_planner::{RequiredCapabilities, RoutePlannerInput};
use crate::llm::routing_policy::RoutingContext;
use crate::settings::RoutingMode;

use super::manager::LlmRuntimeManager;
use super::provider_slots::normalized_provider_pool_order;
use super::types::{
    LlmRuntimeSnapshot, ProviderModelRole, RouteSimulationResult, RouteSimulationScore,
};

impl LlmRuntimeManager {
    pub fn advisor_config_for_messages(
        &self,
        messages: &[crate::llm::ChatMessage],
    ) -> Option<crate::llm::route_planner::AdvisorConfig> {
        let snapshot = self.snapshot();
        let estimated_input_tokens = messages
            .iter()
            .map(|m| (m.estimated_chars() / 4) as u32)
            .sum();
        let has_vision = messages.iter().any(|m| {
            m.attachments
                .iter()
                .any(|a| a.mime_type.starts_with("image/"))
        });
        let last_user_message = messages
            .iter()
            .rev()
            .find(|m| m.role == crate::llm::Role::User)
            .map(|m| m.content.trim().to_string())
            .filter(|msg| !msg.is_empty());

        let ctx = RoutingContext {
            estimated_input_tokens,
            has_vision,
            has_tools: true,
            requires_streaming: false,
            budget_usd: None,
        };

        self.route_decision_for_context(&snapshot, ctx, last_user_message)
            .and_then(|decision| decision.advisor)
    }

    pub fn simulate_route(&self, ctx: RoutingContext) -> (String, String) {
        self.simulate_route_for_prompt(ctx, None)
    }

    pub fn simulate_route_for_prompt(
        &self,
        ctx: RoutingContext,
        prompt: Option<&str>,
    ) -> (String, String) {
        let simulation = self.simulate_route_details(ctx, prompt);
        (simulation.target, simulation.reason)
    }

    pub fn simulate_route_details(
        &self,
        ctx: RoutingContext,
        prompt: Option<&str>,
    ) -> RouteSimulationResult {
        let snapshot = self.snapshot();
        if !snapshot.providers.smart_routing_enabled {
            return RouteSimulationResult {
                target: "primary".to_string(),
                reason: "Routing disabled".to_string(),
                fallback_chain: Vec::new(),
                candidate_list: Vec::new(),
                rejections: Vec::new(),
                score_breakdown: Vec::new(),
                diagnostics: Vec::new(),
            };
        }

        if let Some(decision) =
            self.route_decision_for_context(&snapshot, ctx.clone(), prompt.map(ToOwned::to_owned))
        {
            let reason = if decision.fallbacks.is_empty() {
                decision.reason
            } else {
                format!(
                    "{}. Fallbacks: {}",
                    decision.reason,
                    decision.fallbacks.join(" -> ")
                )
            };
            RouteSimulationResult {
                target: decision.target,
                reason,
                fallback_chain: decision.fallbacks,
                candidate_list: decision.candidate_list,
                rejections: decision
                    .rejections
                    .into_iter()
                    .map(|rejection| format!("{}: {}", rejection.target, rejection.reason))
                    .collect(),
                score_breakdown: decision
                    .score_breakdown
                    .into_iter()
                    .map(|score| RouteSimulationScore {
                        target: score.target,
                        telemetry_key: score.telemetry_key,
                        quality: score.breakdown.quality,
                        cost: score.breakdown.cost,
                        latency: score.breakdown.latency,
                        health: score.breakdown.health,
                        policy_bias: score.breakdown.policy_bias,
                        composite: score.breakdown.composite,
                    })
                    .collect(),
                diagnostics: decision.diagnostics,
            }
        } else {
            RouteSimulationResult {
                target: "primary".to_string(),
                reason: "Planner lock unavailable".to_string(),
                fallback_chain: Vec::new(),
                candidate_list: Vec::new(),
                rejections: Vec::new(),
                score_breakdown: Vec::new(),
                diagnostics: Vec::new(),
            }
        }
    }

    pub(super) fn advisor_status_decision(
        &self,
        snapshot: &LlmRuntimeSnapshot,
    ) -> Option<crate::llm::route_planner::RouteDecision> {
        if snapshot.providers.routing_mode != RoutingMode::AdvisorExecutor
            || !snapshot.providers.smart_routing_enabled
        {
            return None;
        }

        self.route_decision_for_context(
            snapshot,
            RoutingContext {
                estimated_input_tokens: 1024,
                has_vision: false,
                has_tools: true,
                requires_streaming: false,
                budget_usd: None,
            },
            Some("advisor readiness probe".to_string()),
        )
    }

    fn route_decision_for_context(
        &self,
        snapshot: &LlmRuntimeSnapshot,
        ctx: RoutingContext,
        last_user_message: Option<String>,
    ) -> Option<crate::llm::route_planner::RouteDecision> {
        let planner_input = RoutePlannerInput {
            required_capabilities: RequiredCapabilities::from_routing_context(&ctx),
            routing_mode: snapshot.providers.routing_mode,
            routing_context: ctx,
            model_override: None,
            provider_health: self.route_health_snapshot(),
            candidates: self.available_route_candidates(snapshot),
            turn_cost_usd: 0.0,
            budget_utilization: None,
            last_user_message,
            advisor_escalation_prompt: snapshot.providers.advisor_escalation_prompt.clone(),
            primary_provider_preferences: normalized_provider_pool_order(
                &snapshot.providers,
                ProviderModelRole::Primary,
            ),
            cheap_provider_preferences: normalized_provider_pool_order(
                &snapshot.providers,
                ProviderModelRole::Cheap,
            ),
        };

        self.route_planner
            .read()
            .ok()
            .map(|planner| planner.plan(&planner_input, self.routing_policy.read().ok().as_deref()))
    }
}
