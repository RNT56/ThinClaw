//! Shared runtime-manager DTOs and internal value types.
//!
//! Public DTOs (`RuntimeStatus`, `RouteSimulationScore`, `RouteSimulationResult`)
//! are re-exported through the `runtime_manager` façade to preserve external
//! paths. The remaining types are runtime-internal and stay `pub(super)`.

use std::sync::Arc;

use crate::llm::provider::LlmProvider;
use crate::llm::route_planner::CascadePolicy;
use crate::settings::{ProvidersSettings, RoutingMode};

use super::manager::LlmRuntimeManager;

#[derive(Clone)]
pub struct RuntimeStatus {
    pub revision: u64,
    pub last_error: Option<String>,
    pub primary_model: String,
    pub cheap_model: Option<String>,
    pub routing_enabled: bool,
    pub routing_mode: RoutingMode,
    pub tool_phase_synthesis_enabled: bool,
    pub tool_phase_primary_thinking_enabled: bool,
    pub primary_provider: Option<String>,
    pub fallback_chain: Vec<String>,
    /// AdvisorExecutor: max advisor calls per turn.
    pub advisor_max_calls: u32,
    /// AdvisorExecutor: automatic escalation policy.
    pub advisor_auto_escalation_mode: crate::settings::AdvisorAutoEscalationMode,
    /// AdvisorExecutor: custom advisor escalation prompt.
    pub advisor_escalation_prompt: Option<String>,
    /// Whether the advisor lane is currently usable.
    pub advisor_ready: bool,
    /// Why the advisor lane is unavailable, if known.
    pub advisor_disabled_reason: Option<String>,
    /// Resolved executor model spec selected for AdvisorExecutor diagnostics.
    pub executor_target: Option<String>,
    /// Resolved advisor model spec selected for AdvisorExecutor diagnostics.
    pub advisor_target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RouteSimulationScore {
    pub target: String,
    pub telemetry_key: Option<String>,
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
    pub policy_bias: f64,
    pub composite: f64,
}

#[derive(Debug, Clone)]
pub struct RouteSimulationResult {
    pub target: String,
    pub reason: String,
    pub fallback_chain: Vec<String>,
    pub candidate_list: Vec<String>,
    pub rejections: Vec<String>,
    pub score_breakdown: Vec<RouteSimulationScore>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone)]
pub(super) struct LlmRuntimeSnapshot {
    pub(super) config: crate::config::Config,
    pub(super) providers: ProvidersSettings,
    pub(super) llm: Arc<dyn LlmProvider>,
    pub(super) cheap_llm: Option<Arc<dyn LlmProvider>>,
}

pub(super) type AdvisorReadyCallback = Arc<dyn Fn(bool) + Send + Sync>;

#[derive(Clone, Copy)]
pub(super) enum RuntimeProviderRole {
    Primary,
    Cheap,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderModelRole {
    Primary,
    Cheap,
}

impl ProviderModelRole {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Cheap => "cheap",
        }
    }
}

pub(super) struct ResolvedRoute {
    pub(super) provider: Arc<dyn LlmProvider>,
    pub(super) telemetry_key: String,
    /// Planner-decided post-response cascade behavior. `None` for every
    /// non-planner path (model override, kill-switch, fallback).
    pub(super) cascade: CascadePolicy,
    /// When `cascade == InspectAndEscalate` and the resolved `provider` is the
    /// cheap lane, this carries the pre-resolved primary chain to re-issue
    /// against (and its telemetry key) so escalation does not re-run the
    /// planner. `None` otherwise.
    pub(super) escalation: Option<CascadeEscalation>,
}

/// Pre-resolved primary escalation target for a cheap-lane cascade decision.
pub(super) struct CascadeEscalation {
    pub(super) provider: Arc<dyn LlmProvider>,
    pub(super) telemetry_key: String,
}

pub struct RuntimeLlmProvider {
    pub(super) manager: Arc<LlmRuntimeManager>,
    pub(super) role: RuntimeProviderRole,
}
