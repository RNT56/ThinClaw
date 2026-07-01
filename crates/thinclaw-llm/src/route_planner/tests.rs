use super::*;
use thinclaw_llm_core::routing_policy::RouteCandidate;

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
            RouteCandidate::new("primary", Some(30.0)).with_capabilities(
                thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                    supports_streaming: Some(true),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                    supports_thinking: Some(true),
                    ..Default::default()
                },
            ),
            RouteCandidate::new("cheap", Some(1.0)).with_capabilities(
                thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                    supports_streaming: Some(true),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                    supports_thinking: Some(true),
                    ..Default::default()
                },
            ),
        ],
        turn_cost_usd: 0.0,
        budget_utilization: None,
        last_user_message: None,
        advisor_escalation_prompt: None,
        primary_provider_preferences: Vec::new(),
        cheap_provider_preferences: Vec::new(),
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
    input.last_user_message =
        Some("Can you tell me about the differences between these approaches?".to_string());
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
    assert!(d.advisor_ready);
    assert_eq!(d.executor_target.as_deref(), Some("cheap"));
    assert_eq!(d.advisor_target.as_deref(), Some("primary"));
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
    assert!(!d.advisor_ready);
    assert!(d.advisor_disabled_reason.is_some());
}

#[test]
fn advisor_config_max_calls() {
    let p = RoutePlanner::new(true, false, 5);
    let mut input = default_input();
    input.routing_mode = RoutingMode::AdvisorExecutor;
    let d = p.plan(&input, None);
    assert_eq!(d.advisor.as_ref().unwrap().max_advisor_calls, 5);
}

#[test]
fn advisor_executor_falls_back_when_cheap_lane_lacks_required_capability() {
    let p = planner();
    let mut input = default_input();
    input.routing_mode = RoutingMode::AdvisorExecutor;
    input.required_capabilities.tool_use = true;
    input.candidates = vec![
        RouteCandidate::new("primary", Some(30.0)).with_capabilities(
            thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                supports_tools: Some(true),
                ..Default::default()
            },
        ),
        RouteCandidate::new("cheap", Some(1.0)).with_capabilities(
            thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                supports_tools: Some(false),
                ..Default::default()
            },
        ),
    ];

    let decision = p.plan(&input, None);
    assert_eq!(decision.target, "primary");
    assert!(!decision.advisor_ready);
    assert!(
        decision
            .advisor_disabled_reason
            .as_deref()
            .unwrap_or_default()
            .contains("cheap-capable executor")
    );
}

#[test]
fn advisor_executor_rejects_executor_lane_when_required_capability_metadata_is_unknown() {
    let p = planner();
    let mut input = default_input();
    input.routing_mode = RoutingMode::AdvisorExecutor;
    input.required_capabilities.tool_use = true;
    input.candidates = vec![
        RouteCandidate::new("primary", Some(30.0)).with_capabilities(
            thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                supports_tools: Some(true),
                ..Default::default()
            },
        ),
        RouteCandidate::new("cheap", Some(1.0)),
    ];

    let decision = p.plan(&input, None);

    assert_eq!(decision.target, "primary");
    assert!(!decision.advisor_ready);
    assert!(decision.rejections.iter().any(|rejection| {
        rejection.target == "cheap"
            && rejection
                .reason
                .contains("missing verified capability metadata for executor lane")
    }));
}

#[test]
fn advisor_executor_disables_when_executor_and_advisor_resolve_to_same_model() {
    let p = planner();
    let mut input = default_input();
    input.routing_mode = RoutingMode::AdvisorExecutor;
    input.candidates = vec![
        RouteCandidate::new("primary", Some(30.0))
            .with_identity(Some("openai".to_string()), Some("gpt-4o-mini".to_string())),
        RouteCandidate::new("cheap", Some(1.0))
            .with_identity(Some("openai".to_string()), Some("gpt-4o-mini".to_string())),
    ];

    let decision = p.plan(&input, None);
    assert_eq!(decision.target, "primary");
    assert!(!decision.advisor_ready);
    assert!(
        decision
            .advisor_disabled_reason
            .as_deref()
            .unwrap_or_default()
            .contains("same provider/model")
    );
}

#[test]
fn advisor_executor_identity_check_prefers_concrete_slot_targets() {
    let p = planner();
    let mut input = default_input();
    input.routing_mode = RoutingMode::AdvisorExecutor;
    input.candidates = vec![
        RouteCandidate::new("cheap", Some(1.0))
            .with_identity(Some("openai".to_string()), Some("gpt-5.4-mini".to_string())),
        RouteCandidate::new("primary", Some(30.0))
            .with_identity(Some("openai".to_string()), Some("gpt-5.4-mini".to_string())),
        RouteCandidate::new("openai@cheap", Some(1.0))
            .with_identity(Some("openai".to_string()), Some("gpt-5.4-mini".to_string())),
        RouteCandidate::new("openai@primary", Some(30.0))
            .with_identity(Some("openai".to_string()), Some("gpt-5.4".to_string())),
    ];

    let decision = p.plan(&input, None);
    assert!(decision.advisor_ready);
    assert!(decision.advisor_disabled_reason.is_none());
    assert_eq!(decision.executor_target.as_deref(), Some("openai@cheap"));
    assert_eq!(decision.advisor_target.as_deref(), Some("openai@primary"));
}

#[test]
fn advisor_executor_biases_toward_configured_primary_and_cheap_providers() {
    let p = planner();
    let mut input = default_input();
    input.routing_mode = RoutingMode::AdvisorExecutor;
    input.primary_provider_preferences = vec!["openai".to_string(), "anthropic".to_string()];
    input.cheap_provider_preferences = vec!["openai".to_string(), "anthropic".to_string()];
    input.candidates = vec![
        RouteCandidate::new("openai@primary", Some(6.0))
            .with_identity(
                Some("openai".to_string()),
                Some("unknown-primary".to_string()),
            )
            .with_health(Some(0.9))
            .with_capabilities(
                thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                    supports_streaming: Some(true),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                    ..Default::default()
                },
            ),
        RouteCandidate::new("anthropic@primary", Some(6.0))
            .with_identity(
                Some("anthropic".to_string()),
                Some("unknown-primary-alt".to_string()),
            )
            .with_health(Some(1.0))
            .with_capabilities(
                thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                    supports_streaming: Some(true),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                    ..Default::default()
                },
            ),
        RouteCandidate::new("openai@cheap", Some(3.0))
            .with_identity(
                Some("openai".to_string()),
                Some("unknown-cheap".to_string()),
            )
            .with_health(Some(0.9))
            .with_capabilities(
                thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                    supports_streaming: Some(true),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                    ..Default::default()
                },
            ),
        RouteCandidate::new("anthropic@cheap", Some(3.0))
            .with_identity(
                Some("anthropic".to_string()),
                Some("unknown-cheap-alt".to_string()),
            )
            .with_health(Some(1.0))
            .with_capabilities(
                thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                    supports_streaming: Some(true),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                    ..Default::default()
                },
            ),
    ];

    let decision = p.plan(&input, None);
    assert_eq!(decision.executor_target.as_deref(), Some("openai@cheap"));
    assert_eq!(decision.advisor_target.as_deref(), Some("openai@primary"));
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
        supports_streaming: Some(false),
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
    assert!(matches!(result, ScoreOutcome::Rejected(_)));
}

#[test]
fn scorer_fail_open_on_unknown_capability_metadata() {
    let scorer = RouteScorer::new(RoutingWeights::default());
    let caps = ProviderCapabilities::default(); // unknown capability metadata => fail-open
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
    assert!(
        matches!(result, ScoreOutcome::Scored(_)),
        "unknown capability metadata must fail-open"
    );
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
    assert!(matches!(result, ScoreOutcome::Rejected(_)));
}

#[test]
fn scorer_budget_pressure_high() {
    let scorer = RouteScorer::new(RoutingWeights::default());
    let caps = ProviderCapabilities::default();
    let required = RequiredCapabilities::default();

    let normal = match scorer.score(
        &RouteCandidate::new("test", Some(10.0)),
        &caps,
        &required,
        1.0,
        None,
        0.0,
        Some(0.3), // low budget usage
        100,
    ) {
        ScoreOutcome::Scored(score) => score,
        ScoreOutcome::Rejected(reason) => panic!("unexpected rejection: {reason}"),
    };

    let high_pressure = match scorer.score(
        &RouteCandidate::new("test", Some(10.0)),
        &caps,
        &required,
        1.0,
        None,
        0.0,
        Some(0.95), // near budget limit
        100,
    ) {
        ScoreOutcome::Scored(score) => score,
        ScoreOutcome::Rejected(reason) => panic!("unexpected rejection: {reason}"),
    };

    // High budget pressure should increase cost weight, changing composite
    assert!(high_pressure.composite != normal.composite);
}

#[test]
fn scorer_prefers_resolved_model_identity_for_quality() {
    let scorer = RouteScorer::new(RoutingWeights::default());
    let caps = ProviderCapabilities::default();
    let required = RequiredCapabilities::default();

    let high_quality = RouteCandidate::new("openai@primary", Some(30.0))
        .with_identity(Some("openai".to_string()), Some("gpt-4o".to_string()));
    let low_quality = RouteCandidate::new("openai@cheap", Some(1.0))
        .with_identity(Some("openai".to_string()), Some("gpt-4o-mini".to_string()));

    let high = match scorer.score(&high_quality, &caps, &required, 1.0, None, 0.0, None, 256) {
        ScoreOutcome::Scored(score) => score,
        ScoreOutcome::Rejected(reason) => panic!("unexpected rejection: {reason}"),
    };
    let low = match scorer.score(&low_quality, &caps, &required, 1.0, None, 0.0, None, 256) {
        ScoreOutcome::Scored(score) => score,
        ScoreOutcome::Rejected(reason) => panic!("unexpected rejection: {reason}"),
    };

    assert!(
        high.quality > low.quality,
        "expected model identity-aware quality to rank gpt-4o above gpt-4o-mini"
    );
}

// -- Quality scoring --

#[test]
fn quality_score_uses_model_compat_data() {
    let caps = ProviderCapabilities::default();
    let gpt_54 = RouteCandidate::new("openai@primary", Some(17.5))
        .with_identity(Some("openai".to_string()), Some("gpt-5.4".to_string()));
    let gpt_54_mini = RouteCandidate::new("openai@cheap", Some(5.25))
        .with_identity(Some("openai".to_string()), Some("gpt-5.4-mini".to_string()));

    assert!(
        quality_score_for_candidate(&gpt_54, &caps)
            > quality_score_for_candidate(&gpt_54_mini, &caps)
    );
}

#[test]
fn cheap_split_accepts_provider_slot_aliases_for_bias() {
    let p = planner();
    let mut input = default_input();
    input.routing_mode = RoutingMode::CheapSplit;
    input.last_user_message = Some("hello".to_string());
    input.candidates = vec![
        RouteCandidate::new("openai@primary", Some(30.0))
            .with_identity(Some("openai".to_string()), Some("gpt-4o".to_string())),
        RouteCandidate::new("openai@cheap", Some(1.0))
            .with_identity(Some("openai".to_string()), Some("gpt-4o-mini".to_string())),
    ];
    let decision = p.plan(&input, None);
    assert!(
        decision.target == "openai@cheap" || decision.target == "cheap",
        "expected cheap split to favor cheap slot target, got {}",
        decision.target
    );
}

#[test]
fn policy_emits_no_capable_candidate_diagnostic_when_all_hard_rejected() {
    let p = planner();
    let mut input = default_input();
    input.routing_mode = RoutingMode::Policy;
    input.required_capabilities.streaming = true;
    input.candidates = vec![
        RouteCandidate::new("primary", Some(30.0)).with_capabilities(
            thinclaw_llm_core::routing_policy::ProviderCapabilitiesMetadata {
                supports_streaming: Some(false),
                ..Default::default()
            },
        ),
    ];

    let policy = RoutingPolicy::new("primary");
    let decision = p.plan(&input, Some(&policy));
    assert!(
        decision
            .diagnostics
            .iter()
            .any(|d| d.contains("NO_CAPABLE_CANDIDATE")),
        "expected NO_CAPABLE_CANDIDATE diagnostic, got {:?}",
        decision.diagnostics
    );
}

// -- Config validation --

#[test]
fn validate_advisor_without_cheap_model() {
    let settings = thinclaw_settings::ProvidersSettings {
        routing_mode: RoutingMode::AdvisorExecutor,
        ..thinclaw_settings::ProvidersSettings::default()
    };
    let warnings = validate_providers_settings(&settings);
    assert!(warnings.iter().any(|w| w.contains("AdvisorExecutor")));
}

#[test]
fn validate_policy_without_rules() {
    let settings = thinclaw_settings::ProvidersSettings {
        routing_mode: RoutingMode::Policy,
        ..thinclaw_settings::ProvidersSettings::default()
    };
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

// -- Telemetry normalization (Phase 7) --

#[test]
fn canonical_telemetry_key_format() {
    let key = canonical_telemetry_key("primary", "anthropic", "claude-sonnet-4-20250514");
    assert_eq!(key, "primary|anthropic|claude-sonnet-4-20250514");
}

#[test]
fn enrich_telemetry_key_preserves_role() {
    let mut decision = RouteDecision::primary("test");
    enrich_telemetry_key(&mut decision, "openai", "gpt-4o");
    assert_eq!(decision.telemetry_key, "primary|openai|gpt-4o");
}

#[test]
fn enrich_telemetry_key_for_cheap_target() {
    let p = planner();
    let mut input = default_input();
    input.routing_mode = RoutingMode::CheapSplit;
    input.last_user_message = Some("hello".to_string());
    let mut d = p.plan(&input, None);
    // Should be "cheap||" initially
    assert!(d.telemetry_key.starts_with("cheap"));
    enrich_telemetry_key(&mut d, "anthropic", "claude-3-haiku");
    assert_eq!(d.telemetry_key, "cheap|anthropic|claude-3-haiku");
}

// -- Health signal integration (Phase 8) --

#[test]
fn circuit_breaker_health_scores() {
    let closed = CircuitBreakerHealthProbe::new("test", CircuitBreakerState::Closed);
    assert_eq!(closed.health_score(), 1.0);

    let half_open = CircuitBreakerHealthProbe::new("test", CircuitBreakerState::HalfOpen);
    assert_eq!(half_open.health_score(), 0.5);

    let open = CircuitBreakerHealthProbe::new("test", CircuitBreakerState::Open);
    assert_eq!(open.health_score(), 0.0);

    let unknown = CircuitBreakerHealthProbe::new("test", CircuitBreakerState::Unknown);
    assert_eq!(unknown.health_score(), 0.8);
}

#[test]
fn latency_weighted_health_no_latency() {
    let probe = LatencyWeightedHealthProbe::new("test", CircuitBreakerState::Closed, None);
    assert_eq!(probe.health_score(), 1.0);
}

#[test]
fn latency_weighted_health_low_latency() {
    let probe = LatencyWeightedHealthProbe::new("test", CircuitBreakerState::Closed, Some(500.0));
    assert_eq!(probe.health_score(), 1.0); // No penalty below 2000ms
}

#[test]
fn latency_weighted_health_high_latency() {
    let probe = LatencyWeightedHealthProbe::new("test", CircuitBreakerState::Closed, Some(5500.0));
    let score = probe.health_score();
    assert!(score < 0.8, "High latency should penalize score: {}", score);
    assert!(score >= 0.1, "Score should never drop below 0.1: {}", score);
}

#[test]
fn latency_weighted_health_open_circuit_ignores_latency() {
    let probe = LatencyWeightedHealthProbe::new("test", CircuitBreakerState::Open, Some(100.0));
    assert_eq!(probe.health_score(), 0.0); // Open circuit = 0 regardless
}

#[test]
fn build_health_map_from_probes() {
    let probes: Vec<Box<dyn HealthProbe>> = vec![
        Box::new(CircuitBreakerHealthProbe::new(
            "primary",
            CircuitBreakerState::Closed,
        )),
        Box::new(CircuitBreakerHealthProbe::new(
            "cheap",
            CircuitBreakerState::HalfOpen,
        )),
    ];
    let map = build_health_map(&probes);
    assert_eq!(map.get("primary"), Some(&1.0));
    assert_eq!(map.get("cheap"), Some(&0.5));
}
