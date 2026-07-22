//! `RuntimeLlmProvider`: the `LlmProvider` adapter that fronts the runtime
//! manager. It resolves each request to a concrete provider (model override,
//! kill-switch, planner-driven routing, or fallback), shapes the outgoing
//! request metadata, records route outcomes, and drives cheap-lane cascade
//! escalation.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::error::LlmError;
use crate::llm::cascade::response_is_uncertain;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamChunkStream,
    StreamSupport, TokenCaptureSupport, ToolCompletionRequest, ToolCompletionResponse,
};
use crate::llm::route_planner::{
    CascadePolicy, RequiredCapabilities, RoutePlannerInput, log_routing_decision,
};
use crate::llm::routing_policy::RoutingContext;
use crate::llm::usage_tracking::{
    USAGE_TRACKING_ENDPOINT_TYPE_KEY, USAGE_TRACKING_TELEMETRY_KEY, USAGE_TRACKING_WORKLOAD_TAG_KEY,
};

use super::manager::LlmRuntimeManager;
use super::provider_build::routing_kill_switch_enabled;
use super::provider_slots::normalized_provider_pool_order;
use super::types::{
    CascadeEscalation, ProviderModelRole, ResolvedRoute, RuntimeLlmProvider, RuntimeProviderRole,
};

impl RuntimeLlmProvider {
    pub(super) fn new(manager: Arc<LlmRuntimeManager>, role: RuntimeProviderRole) -> Self {
        Self { manager, role }
    }

    fn current_provider(&self) -> Arc<dyn LlmProvider> {
        let snapshot = self.manager.snapshot();
        match self.role {
            RuntimeProviderRole::Primary => snapshot.llm,
            RuntimeProviderRole::Cheap => snapshot.cheap_llm.unwrap_or(snapshot.llm),
        }
    }

    fn routing_context(
        &self,
        messages: &[crate::llm::ChatMessage],
        has_tools: bool,
        requires_streaming: bool,
    ) -> (RoutingContext, Option<String>) {
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
            .find(|m| m.is_user_instruction())
            .map(|m| m.content.trim().to_string())
            .filter(|msg| !msg.is_empty());

        (
            RoutingContext {
                estimated_input_tokens,
                has_vision,
                has_tools,
                requires_streaming,
                budget_usd: None,
            },
            last_user_message,
        )
    }

    fn provider_for_request(
        &self,
        requested_model: Option<&str>,
        routing_context: Option<&RoutingContext>,
        last_user_message: Option<String>,
    ) -> Result<ResolvedRoute, LlmError> {
        let snapshot = self.manager.snapshot();

        if let Some(model) = requested_model {
            let provider = self.manager.provider_for_model_spec(model, &snapshot)?;
            if routing_context
                .map(|ctx| ctx.requires_streaming)
                .unwrap_or(false)
                && !provider
                    .stream_support_for_model(Some(model))
                    .is_available()
            {
                return Err(LlmError::StreamingUnsupported {
                    provider: provider.model_name().to_string(),
                    model: model.trim().to_string(),
                });
            }
            let telemetry_key = format!("unknown|unknown|{}", model.trim());
            return Ok(ResolvedRoute {
                provider,
                telemetry_key,
                cascade: CascadePolicy::None,
                escalation: None,
            });
        }

        if routing_kill_switch_enabled() {
            let (provider, target) = match self.role {
                RuntimeProviderRole::Primary => (Arc::clone(&snapshot.llm), "primary"),
                RuntimeProviderRole::Cheap => (
                    snapshot
                        .cheap_llm
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| Arc::clone(&snapshot.llm)),
                    "cheap",
                ),
            };
            let telemetry_key = self
                .manager
                .resolve_telemetry_key_for_target(target, &snapshot)
                .unwrap_or_else(|| format!("killswitch|runtime|{target}"));
            return Ok(ResolvedRoute {
                provider,
                telemetry_key,
                cascade: CascadePolicy::None,
                escalation: None,
            });
        }

        // ── RoutePlanner-driven routing (Phase 6b cutover) ──
        if snapshot.providers.smart_routing_enabled
            && let Some(ctx) = routing_context
        {
            let planner_input = RoutePlannerInput {
                required_capabilities: RequiredCapabilities::from_routing_context(ctx),
                routing_mode: snapshot.providers.routing_mode,
                routing_context: ctx.clone(),
                model_override: None,
                provider_health: self.manager.route_health_snapshot(),
                candidates: self.manager.available_route_candidates(&snapshot),
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

            if let Ok(guard) = self.manager.route_planner.read() {
                let mut decision = guard.plan(
                    &planner_input,
                    self.manager.routing_policy.read().ok().as_deref(),
                );

                // Resolve the planner target to a provider
                let primary_providers = if decision.fallbacks.is_empty() {
                    vec![decision.target.clone()]
                } else {
                    let mut targets = vec![decision.target.clone()];
                    for fb in &decision.fallbacks {
                        if !targets.contains(fb) {
                            targets.push(fb.clone());
                        }
                    }
                    targets
                };

                let provider = self
                    .manager
                    .provider_chain_for_targets(&primary_providers, &snapshot)?;
                let telemetry_key = self
                    .manager
                    .resolve_telemetry_key_for_target(&decision.target, &snapshot)
                    .or_else(|| {
                        if decision.telemetry_key.contains('|') {
                            Some(decision.telemetry_key.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| format!("unknown|unknown|{}", decision.target));
                decision.telemetry_key = telemetry_key.clone();
                log_routing_decision(&decision, snapshot.providers.routing_mode.as_str());

                // ── Cascade (CheapSplit inspect-and-escalate) ──
                // The planner decides `InspectAndEscalate` for Moderate-complexity
                // CheapSplit turns when cascade is enabled. Escalation only makes
                // sense when the selected lane is the cheap one; if the planner
                // already picked primary there is nothing to escalate to. We
                // pre-resolve a pure-primary chain here so the completion path can
                // re-issue without re-running the planner.
                let target_is_cheap =
                    decision.target == "cheap" || decision.target.ends_with("@cheap");
                let escalation =
                    if decision.cascade == CascadePolicy::InspectAndEscalate && target_is_cheap {
                        match self
                            .manager
                            .provider_chain_for_targets(&["primary".to_string()], &snapshot)
                        {
                            Ok(primary_provider) => {
                                let escalation_key = self
                                    .manager
                                    .resolve_telemetry_key_for_target("primary", &snapshot)
                                    .unwrap_or_else(|| "unknown|unknown|primary".to_string());
                                Some(CascadeEscalation {
                                    provider: primary_provider,
                                    telemetry_key: escalation_key,
                                })
                            }
                            Err(err) => {
                                // Without a usable primary chain there is nothing to
                                // escalate to; degrade to the cheap result rather than
                                // failing the turn.
                                tracing::warn!(
                                    error = %err,
                                    "Cascade escalation disabled: no usable primary chain"
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };
                let cascade = if escalation.is_some() {
                    CascadePolicy::InspectAndEscalate
                } else {
                    CascadePolicy::None
                };

                return Ok(ResolvedRoute {
                    provider,
                    telemetry_key,
                    cascade,
                    escalation,
                });
            }
        }

        // Fallback: no routing context available or planner lock failed
        let provider = match self.role {
            RuntimeProviderRole::Primary => snapshot.llm,
            RuntimeProviderRole::Cheap => snapshot.cheap_llm.unwrap_or(snapshot.llm),
        };
        Ok(ResolvedRoute {
            provider,
            telemetry_key: "unknown|unknown|runtime_fallback".to_string(),
            cascade: CascadePolicy::None,
            escalation: None,
        })
    }

    pub(super) fn resolved_completion_request(
        mut request: CompletionRequest,
        telemetry_key: &str,
        workload_tag: &str,
    ) -> CompletionRequest {
        // The runtime resolves `request.model` to a concrete provider before
        // delegating, so downstream adapters should not see a stale override.
        let endpoint_type = if request.model.is_some() {
            "model_override"
        } else {
            "runtime_routed"
        };
        request.model = None;
        request.metadata.insert(
            USAGE_TRACKING_TELEMETRY_KEY.to_string(),
            telemetry_key.to_string(),
        );
        request
            .metadata
            .entry(USAGE_TRACKING_ENDPOINT_TYPE_KEY.to_string())
            .or_insert_with(|| endpoint_type.to_string());
        request
            .metadata
            .entry(USAGE_TRACKING_WORKLOAD_TAG_KEY.to_string())
            .or_insert_with(|| workload_tag.to_string());
        request
    }

    pub(super) fn resolved_tool_completion_request(
        mut request: ToolCompletionRequest,
        telemetry_key: &str,
        workload_tag: &str,
    ) -> ToolCompletionRequest {
        // The runtime resolves `request.model` to a concrete provider before
        // delegating, so downstream adapters should not see a stale override.
        let endpoint_type = if request.model.is_some() {
            "model_override"
        } else {
            "runtime_routed"
        };
        request.model = None;
        request.metadata.insert(
            USAGE_TRACKING_TELEMETRY_KEY.to_string(),
            telemetry_key.to_string(),
        );
        request
            .metadata
            .entry(USAGE_TRACKING_ENDPOINT_TYPE_KEY.to_string())
            .or_insert_with(|| endpoint_type.to_string());
        request
            .metadata
            .entry(USAGE_TRACKING_WORKLOAD_TAG_KEY.to_string())
            .or_insert_with(|| workload_tag.to_string());
        request
    }
}

#[async_trait]
impl LlmProvider for RuntimeLlmProvider {
    fn model_name(&self) -> &str {
        "runtime"
    }

    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        self.current_provider().cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (ctx, last_user_message) = self.routing_context(&request.messages, false, false);
        let route =
            self.provider_for_request(request.model.as_deref(), Some(&ctx), last_user_message)?;

        // When the planner selected the cheap lane with inspect-and-escalate
        // cascade, keep a clone of the request so we can re-issue against the
        // pre-resolved primary chain if the cheap response looks uncertain.
        let cascade_active =
            route.cascade == CascadePolicy::InspectAndEscalate && route.escalation.is_some();
        let escalation_request = if cascade_active {
            Some(request.clone())
        } else {
            None
        };

        let start = Instant::now();
        let result = route
            .provider
            .complete(Self::resolved_completion_request(
                request,
                &route.telemetry_key,
                "chat_completion",
            ))
            .await;
        self.manager
            .record_route_outcome(&route.telemetry_key, start.elapsed(), result.is_ok());

        // Cascade escalation: if the cheap response is uncertain, re-issue
        // against the primary chain. Mirrors the legacy SmartRoutingProvider
        // behavior, now driven by the planner's decision.
        if cascade_active
            && let Ok(ref response) = result
            && response_is_uncertain(response)
            && let (Some(escalation), Some(escalation_request)) =
                (route.escalation.as_ref(), escalation_request)
        {
            tracing::info!(
                cheap_telemetry_key = %route.telemetry_key,
                primary_telemetry_key = %escalation.telemetry_key,
                "Cascade: escalating to primary (cheap model response uncertain)"
            );
            let escalation_start = Instant::now();
            let escalation_result = escalation
                .provider
                .complete(Self::resolved_completion_request(
                    escalation_request,
                    &escalation.telemetry_key,
                    "chat_completion",
                ))
                .await;
            self.manager.record_route_outcome(
                &escalation.telemetry_key,
                escalation_start.elapsed(),
                escalation_result.is_ok(),
            );
            return escalation_result;
        }

        result
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (ctx, last_user_message) = self.routing_context(&request.messages, true, false);
        let route =
            self.provider_for_request(request.model.as_deref(), Some(&ctx), last_user_message)?;
        let start = Instant::now();
        let result = route
            .provider
            .complete_with_tools(Self::resolved_tool_completion_request(
                request,
                &route.telemetry_key,
                "tool_completion",
            ))
            .await;
        self.manager
            .record_route_outcome(&route.telemetry_key, start.elapsed(), result.is_ok());
        result
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let (ctx, last_user_message) = self.routing_context(&request.messages, false, true);
        let route =
            self.provider_for_request(request.model.as_deref(), Some(&ctx), last_user_message)?;
        let start = Instant::now();
        let result = route
            .provider
            .complete_stream(Self::resolved_completion_request(
                request,
                &route.telemetry_key,
                "stream_completion",
            ))
            .await;
        self.manager
            .record_route_outcome(&route.telemetry_key, start.elapsed(), result.is_ok());
        result
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let (ctx, last_user_message) = self.routing_context(&request.messages, true, true);
        let route =
            self.provider_for_request(request.model.as_deref(), Some(&ctx), last_user_message)?;
        let start = Instant::now();
        let result = route
            .provider
            .complete_stream_with_tools(Self::resolved_tool_completion_request(
                request,
                &route.telemetry_key,
                "tool_stream_completion",
            ))
            .await;
        self.manager
            .record_route_outcome(&route.telemetry_key, start.elapsed(), result.is_ok());
        result
    }

    fn supports_streaming(&self) -> bool {
        self.current_provider().supports_streaming()
    }

    fn stream_support(&self) -> StreamSupport {
        self.current_provider().stream_support()
    }

    fn stream_support_for_model(&self, requested_model: Option<&str>) -> StreamSupport {
        let snapshot = self.manager.snapshot();
        self.provider_for_request(requested_model, None, None)
            .map(|route| route.provider.stream_support_for_model(requested_model))
            .unwrap_or_else(|_| match self.role {
                RuntimeProviderRole::Primary => snapshot.llm.stream_support(),
                RuntimeProviderRole::Cheap => {
                    snapshot.cheap_llm.unwrap_or(snapshot.llm).stream_support()
                }
            })
    }

    fn supports_streaming_for_model(&self, requested_model: Option<&str>) -> bool {
        self.stream_support_for_model(requested_model).is_native()
    }

    fn token_capture_support(&self) -> TokenCaptureSupport {
        self.current_provider().token_capture_support()
    }

    fn token_capture_support_for_model(
        &self,
        requested_model: Option<&str>,
    ) -> TokenCaptureSupport {
        let snapshot = self.manager.snapshot();
        self.provider_for_request(requested_model, None, None)
            .map(|route| {
                route
                    .provider
                    .token_capture_support_for_model(requested_model)
            })
            .unwrap_or_else(|_| match self.role {
                RuntimeProviderRole::Primary => snapshot.llm.token_capture_support(),
                RuntimeProviderRole::Cheap => snapshot
                    .cheap_llm
                    .unwrap_or(snapshot.llm)
                    .token_capture_support(),
            })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let snapshot = self.manager.snapshot();
        let mut models = BTreeSet::new();

        models.insert(snapshot.llm.active_model_name());
        if let Some(cheap) = snapshot.cheap_llm {
            models.insert(cheap.active_model_name());
        }

        if let Some(primary) = snapshot.providers.primary.as_deref()
            && let Some(model) = snapshot.providers.primary_model.as_deref()
        {
            models.insert(format!("{primary}/{model}"));
        }

        for entry in &snapshot.providers.fallback_chain {
            models.insert(entry.clone());
        }
        if let Some(ref cheap) = snapshot.providers.cheap_model {
            models.insert(cheap.clone());
        }
        for (slug, slots) in &snapshot.providers.provider_models {
            if let Some(model) = slots.primary.as_deref() {
                models.insert(format!("{slug}/{model}"));
            }
            if let Some(model) = slots.cheap.as_deref() {
                models.insert(format!("{slug}/{model}"));
            }
        }
        for (slug, allowed) in &snapshot.providers.allowed_models {
            for model in allowed {
                models.insert(format!("{slug}/{model}"));
            }
        }

        Ok(models.into_iter().collect())
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.current_provider().model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        if let Some(model) = requested_model {
            return model.to_string();
        }
        self.active_model_name()
    }

    fn active_model_name(&self) -> String {
        let snapshot = self.manager.snapshot();
        match self.role {
            RuntimeProviderRole::Primary => snapshot
                .providers
                .primary
                .as_ref()
                .zip(snapshot.providers.primary_model.as_ref())
                .map(|(provider, model)| format!("{provider}/{model}"))
                .unwrap_or_else(|| snapshot.llm.active_model_name()),
            RuntimeProviderRole::Cheap => snapshot
                .providers
                .cheap_model
                .clone()
                .or_else(|| snapshot.cheap_llm.map(|llm| llm.active_model_name()))
                .unwrap_or_else(|| snapshot.llm.active_model_name()),
        }
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        Err(LlmError::RequestFailed {
            provider: "runtime".to_string(),
            reason: format!(
                "Runtime model switching is conversation-scoped in ThinClaw. Use llm_select or a per-request model override instead of set_model('{}').",
                model
            ),
        })
    }
}
