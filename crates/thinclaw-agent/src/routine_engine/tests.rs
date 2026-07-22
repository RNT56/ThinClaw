use super::*;

fn test_routine(name: &str, trigger: Trigger) -> Routine {
    Routine {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: "test routine".to_string(),
        user_id: "default".to_string(),
        actor_id: "default".to_string(),
        enabled: true,
        trigger,
        action: crate::routine::RoutineAction::Lightweight {
            prompt: "run".to_string(),
            context_paths: Vec::new(),
            max_tokens: 32,
        },
        guardrails: crate::routine::RoutineGuardrails::default(),
        notify: NotifyConfig::default(),
        policy: Default::default(),
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        config_version: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn test_event() -> RoutineEvent {
    RoutineEvent {
        id: Uuid::new_v4(),
        principal_id: "default".to_string(),
        actor_id: "default".to_string(),
        channel: "slack".to_string(),
        event_type: "reaction_added".to_string(),
        raw_sender_id: "sender-a".to_string(),
        conversation_scope_id: Uuid::new_v4().to_string(),
        stable_external_conversation_key: "test://routine-event".to_string(),
        idempotency_key: "event:slack:default:default:reaction_added:message-1".to_string(),
        content: "deploy".to_string(),
        content_hash: content_hash("deploy").to_string(),
        metadata: serde_json::json!({
            "message_id": "message-1",
            "tag": "deploy",
            "flags": ["urgent", "audit"]
        }),
        status: RoutineEventStatus::Pending,
        diagnostics: serde_json::json!({}),
        claimed_by: None,
        claimed_at: None,
        lease_expires_at: None,
        processed_at: None,
        error_message: None,
        matched_routines: 0,
        fired_routines: 0,
        attempt_count: 0,
        created_at: Utc::now(),
    }
}

fn test_scheduled_trigger(routine_id: Uuid, trigger_kind: RoutineTriggerKind) -> RoutineTrigger {
    let now = Utc::now();
    RoutineTrigger {
        id: Uuid::new_v4(),
        routine_id,
        trigger_kind,
        trigger_label: Some("every 1h".to_string()),
        due_at: now - chrono::Duration::hours(1),
        status: RoutineTriggerStatus::Processing,
        decision: None,
        active_key: Some(format!("routine:{routine_id}:{trigger_kind}")),
        idempotency_key: format!("routine:{routine_id}:{trigger_kind}:{}:v1", now.timestamp()),
        claimed_by: Some("worker".to_string()),
        claimed_at: Some(now),
        lease_expires_at: None,
        processed_at: None,
        error_message: None,
        diagnostics: serde_json::json!({}),
        coalesced_count: 0,
        backlog_collapsed: true,
        routine_config_version: 1,
        created_at: now,
    }
}

#[test]
fn event_cache_ordering_prefers_priority_then_stable_ties() {
    let base_created_at = Utc::now();
    let mut low = test_routine(
        "low",
        Trigger::Event {
            channel: None,
            event_type: None,
            actor: None,
            metadata: None,
            pattern: String::new(),
            priority: 1,
        },
    );
    low.created_at = base_created_at;
    let mut high_newer = test_routine(
        "high-b",
        Trigger::Event {
            channel: None,
            event_type: None,
            actor: None,
            metadata: None,
            pattern: String::new(),
            priority: 10,
        },
    );
    high_newer.created_at = base_created_at + chrono::Duration::seconds(1);
    let mut high_older = test_routine(
        "high-a",
        Trigger::Event {
            channel: None,
            event_type: None,
            actor: None,
            metadata: None,
            pattern: String::new(),
            priority: 10,
        },
    );
    high_older.created_at = base_created_at;

    let mut routines = [low, high_newer, high_older];
    routines.sort_by(compare_event_cache_routines);

    assert_eq!(routines[0].name, "high-a");
    assert_eq!(routines[1].name, "high-b");
    assert_eq!(routines[2].name, "low");
}

#[test]
fn event_filter_policy_matches_structured_event_and_ignores_pattern_miss() {
    let routine = test_routine(
        "structured-event",
        Trigger::Event {
            channel: Some("slack".to_string()),
            event_type: Some("reaction_added".to_string()),
            actor: Some("sender-a".to_string()),
            metadata: Some(serde_json::json!({"flags": ["urgent"]})),
            pattern: "deploy".to_string(),
            priority: 0,
        },
    );
    let event = test_event();

    let matched = evaluate_routine_event_filters(&routine, &event, true, Utc::now(), 60);
    assert_eq!(
        matched,
        RoutineEventFilterOutcome::Matched {
            trigger_key: event_run_trigger_key(&event)
        }
    );

    let ignored = evaluate_routine_event_filters(&routine, &event, false, Utc::now(), 60);
    assert_eq!(
        ignored,
        RoutineEventFilterOutcome::Ignored {
            decision: RoutineEventDecision::IgnoredPattern,
            reason: "pattern did not match event content".to_string()
        }
    );
}

#[test]
fn event_dispatch_policy_preserves_decision_order_and_deferrals() {
    let duplicate = decide_routine_event_dispatch(true, false, false, false);
    assert_eq!(duplicate.decision, RoutineEventDecision::SkippedDuplicate);
    assert!(!duplicate.deferred);
    assert!(!duplicate.should_fire);

    let routine_full = decide_routine_event_dispatch(false, true, false, true);
    assert_eq!(
        routine_full.decision,
        RoutineEventDecision::DeferredConcurrency
    );
    assert!(routine_full.deferred);

    let fired = decide_routine_event_dispatch(false, true, true, true);
    assert_eq!(fired.decision, RoutineEventDecision::Fired);
    assert!(fired.should_fire);
}

#[test]
fn scheduled_trigger_policy_handles_skip_duplicate_and_deferrals() {
    let now = Utc::now();
    let mut routine = test_routine(
        "scheduled",
        Trigger::Cron {
            schedule: "every 1h".to_string(),
        },
    );
    let trigger = test_scheduled_trigger(routine.id, RoutineTriggerKind::Cron);

    let duplicate = decide_claimed_scheduled_trigger(ClaimedScheduledTriggerDecisionInput {
        routine: &routine,
        trigger: &trigger,
        duplicate_exists: true,
        cooldown_allowed: false,
        routine_capacity_available: false,
        global_capacity_available: false,
        user_timezone: None,
        now,
    })
    .unwrap();
    assert_eq!(duplicate.decision, RoutineTriggerDecision::SkippedDuplicate);
    assert_eq!(duplicate.action, ScheduledTriggerAction::Complete);

    routine.policy.catch_up_mode = RoutineCatchUpMode::Skip;
    let skip = decide_claimed_scheduled_trigger(ClaimedScheduledTriggerDecisionInput {
        routine: &routine,
        trigger: &trigger,
        duplicate_exists: false,
        cooldown_allowed: true,
        routine_capacity_available: true,
        global_capacity_available: true,
        user_timezone: None,
        now,
    })
    .unwrap();
    assert_eq!(skip.decision, RoutineTriggerDecision::SkippedCatchUp);
    assert_eq!(skip.action, ScheduledTriggerAction::Complete);
    assert!(skip.next_fire_at.is_some_and(|next| next > now));

    routine.policy.catch_up_mode = RoutineCatchUpMode::RunOnceNow;
    let cooldown = decide_claimed_scheduled_trigger(ClaimedScheduledTriggerDecisionInput {
        routine: &routine,
        trigger: &trigger,
        duplicate_exists: false,
        cooldown_allowed: false,
        routine_capacity_available: true,
        global_capacity_available: true,
        user_timezone: None,
        now,
    })
    .unwrap();
    assert_eq!(cooldown.decision, RoutineTriggerDecision::DeferredCooldown);
    assert_eq!(cooldown.action, ScheduledTriggerAction::Release);

    let global_full = decide_claimed_scheduled_trigger(ClaimedScheduledTriggerDecisionInput {
        routine: &routine,
        trigger: &trigger,
        duplicate_exists: false,
        cooldown_allowed: true,
        routine_capacity_available: true,
        global_capacity_available: false,
        user_timezone: None,
        now,
    })
    .unwrap();
    assert_eq!(
        global_full.decision,
        RoutineTriggerDecision::DeferredGlobalCapacity
    );
    assert_eq!(global_full.action, ScheduledTriggerAction::Release);
}

#[test]
fn scheduled_trigger_policy_exempts_system_events_from_capacity_gates() {
    let routine = test_routine(
        "system-event",
        Trigger::SystemEvent {
            message: "check".to_string(),
            schedule: Some("every 1h".to_string()),
        },
    );
    let trigger = test_scheduled_trigger(routine.id, RoutineTriggerKind::SystemEvent);

    let plan = decide_claimed_scheduled_trigger(ClaimedScheduledTriggerDecisionInput {
        routine: &routine,
        trigger: &trigger,
        duplicate_exists: false,
        cooldown_allowed: true,
        routine_capacity_available: false,
        global_capacity_available: false,
        user_timezone: None,
        now: Utc::now(),
    })
    .unwrap();

    assert_eq!(plan.decision, RoutineTriggerDecision::Fired);
    assert_eq!(plan.action, ScheduledTriggerAction::Dispatch);
}

#[test]
fn runtime_update_policy_advances_runs_and_marks_dispatched_state() {
    let now = Utc::now();
    let mut routine = test_routine(
        "runtime",
        Trigger::Cron {
            schedule: "every 1h".to_string(),
        },
    );
    routine.run_count = 3;
    routine.consecutive_failures = 2;
    let run_id = Uuid::new_v4();

    let dispatched =
        routine_runtime_update_for_run(&routine, run_id, RunStatus::Running, None, now).unwrap();
    assert_eq!(dispatched.run_count, 4);
    assert_eq!(dispatched.consecutive_failures, 2);
    assert!(crate::routine::routine_state_has_runtime_advance_for_run(
        &dispatched.state,
        run_id
    ));

    let failed =
        routine_runtime_update_for_run(&routine, run_id, RunStatus::Failed, None, now).unwrap();
    assert_eq!(failed.run_count, 4);
    assert_eq!(failed.consecutive_failures, 3);
    assert_eq!(failed.state, routine.state);

    let ok = routine_runtime_update_for_run(&routine, run_id, RunStatus::Ok, None, now).unwrap();
    assert_eq!(ok.consecutive_failures, 0);
}

#[test]
fn failure_backoff_kicks_in_after_threshold_and_grows() {
    assert_eq!(routine_failure_backoff(0), None);
    assert_eq!(routine_failure_backoff(2), None);
    let third = routine_failure_backoff(3).expect("backoff at threshold");
    let sixth = routine_failure_backoff(6).expect("backoff past schedule");
    assert!(sixth > third);
    assert!(!routine_should_auto_disable(
        ROUTINE_AUTO_DISABLE_THRESHOLD - 1
    ));
    assert!(routine_should_auto_disable(ROUTINE_AUTO_DISABLE_THRESHOLD));
}

#[test]
fn repeated_failures_push_next_fire_beyond_schedule() {
    let now = Utc::now();
    let mut routine = test_routine(
        "backoff",
        Trigger::Cron {
            schedule: "every 1m".to_string(),
        },
    );
    routine.consecutive_failures = 4; // this failure makes it 5 → 1h backoff
    let run_id = Uuid::new_v4();

    let failed =
        routine_runtime_update_for_run(&routine, run_id, RunStatus::Failed, None, now).unwrap();
    assert_eq!(failed.consecutive_failures, 5);
    let next_fire = failed.next_fire_at.expect("cron routine has next fire");
    assert!(next_fire >= now + chrono::Duration::minutes(59));

    // Success resets the counter and the schedule is not pushed out.
    let ok = routine_runtime_update_for_run(&routine, run_id, RunStatus::Ok, None, now).unwrap();
    let ok_fire = ok.next_fire_at.expect("cron routine has next fire");
    assert!(ok_fire < now + chrono::Duration::minutes(5));
}

#[test]
fn queue_drain_policy_continues_only_on_full_batches() {
    assert!(should_continue_queue_drain(64, 64, 1, 4));
    assert!(!should_continue_queue_drain(64, 64, 4, 4));
    assert!(!should_continue_queue_drain(63, 64, 1, 4));
    assert!(!should_continue_queue_drain(0, 0, 0, 4));
}

#[test]
fn routine_queue_retry_delay_is_exponential_and_capped() {
    assert_eq!(routine_queue_retry_delay(1), Duration::from_secs(1));
    assert_eq!(routine_queue_retry_delay(2), Duration::from_secs(2));
    assert_eq!(routine_queue_retry_delay(5), Duration::from_secs(16));
    assert_eq!(routine_queue_retry_delay(32), Duration::from_secs(30));
}

#[test]
fn event_attempt_policy_dead_letters_only_at_positive_ceiling() {
    assert!(!routine_event_attempts_exhausted(0, 3));
    assert!(!routine_event_attempts_exhausted(2, 3));
    assert!(routine_event_attempts_exhausted(3, 3));
    assert!(routine_event_attempts_exhausted(4, 3));
    assert!(!routine_event_attempts_exhausted(4, 0));
}

#[test]
fn routine_event_fairness_key_falls_back_to_scope_then_sender() {
    let mut event = test_event();
    event.stable_external_conversation_key = String::new();
    event.conversation_scope_id = "scope-a".to_string();
    assert_eq!(
        routine_event_fairness_key(&event),
        "default:default:slack:scope-a"
    );

    event.conversation_scope_id = String::new();
    event.raw_sender_id = "sender-fallback".to_string();
    assert_eq!(
        routine_event_fairness_key(&event),
        "default:default:slack:sender-fallback"
    );
}

#[test]
fn routine_event_batches_are_fairly_interleaved_by_source() {
    fn event_for(source: &str, sequence: usize) -> RoutineEvent {
        let mut event = test_event();
        event.id = Uuid::from_u128(sequence as u128 + 1);
        event.stable_external_conversation_key = format!("source://{source}");
        event.content = format!("{source}-{sequence}");
        event.content_hash = content_hash(&event.content).to_string();
        event.idempotency_key = format!("event:{source}:{sequence}");
        event.created_at = Utc::now() + chrono::Duration::milliseconds(sequence as i64);
        event
    }

    let ordered = fair_interleave_routine_events(vec![
        event_for("a", 0),
        event_for("a", 1),
        event_for("a", 2),
        event_for("b", 3),
        event_for("c", 4),
        event_for("b", 5),
    ]);
    let contents = ordered
        .into_iter()
        .map(|event| event.content)
        .collect::<Vec<_>>();

    assert_eq!(contents, vec!["a-0", "b-3", "c-4", "a-1", "b-5", "a-2"]);
}

#[test]
fn routine_event_fairness_throughput_smoke() {
    let events = (0..10_000)
        .map(|sequence| {
            let mut event = test_event();
            event.id = Uuid::from_u128(sequence as u128 + 1);
            event.stable_external_conversation_key = format!("source://{}", sequence % 256);
            event
        })
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let ordered = fair_interleave_routine_events(events);

    assert_eq!(ordered.len(), 10_000);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "fair queue interleave exceeded the debug-build timing budget"
    );
}

#[test]
fn sanitize_routine_name_replaces_path_unsafe_chars() {
    assert_eq!(
        sanitize_routine_name("daily/profile sync"),
        "daily_profile_sync"
    );
}

#[test]
fn desktop_capability_detection_matches_known_tools() {
    assert!(routine_requests_desktop_capabilities(Some(&[
        "shell".to_string(),
        "desktop_screen".to_string(),
    ])));
    assert!(!routine_requests_desktop_capabilities(Some(&[
        "shell".to_string()
    ])));
    assert!(!routine_requests_desktop_capabilities(None));
}

#[test]
fn runtime_summary_reports_explicit_empty_grants() {
    let summary = summarize_runtime_capabilities(
        ToolProfile::ExplicitOnly,
        None,
        Some(&["skill-a".to_string()]),
    );

    assert_eq!(
        summary,
        "profile `explicit_only` | tool grants: none | skill grants: skill-a"
    );
}

#[test]
fn truncate_preserves_utf8_boundaries() {
    assert_eq!(truncate("abécd", 4), "abé...");
}

#[test]
fn event_idempotency_prefers_external_message_id() {
    let id = Uuid::new_v4();
    let key = routine_event_idempotency_key(
        "mail",
        "principal",
        "actor",
        "message",
        &serde_json::json!({ "external_message_id": "abc" }),
        id,
    );
    assert_eq!(key, "event:mail:principal:actor:message:abc");
}

#[test]
fn event_cache_refresh_policy_checks_empty_ttl_and_version() {
    let now = Utc::now();

    assert!(should_refresh_event_cache(false, None, 1, Some(1), 60, now));
    assert!(should_refresh_event_cache(
        false,
        Some(now - chrono::Duration::seconds(61)),
        1,
        Some(1),
        60,
        now,
    ));
    assert!(should_refresh_event_cache(
        false,
        Some(now),
        1,
        Some(2),
        60,
        now,
    ));
    assert!(!should_refresh_event_cache(
        false,
        Some(now),
        1,
        Some(1),
        60,
        now,
    ));
}

#[test]
fn active_hour_policy_handles_wrapping_windows() {
    assert!(active_hour_allows(10, 9, 17));
    assert!(!active_hour_allows(17, 9, 17));
    assert!(active_hour_allows(23, 22, 6));
    assert!(active_hour_allows(2, 22, 6));
    assert!(!active_hour_allows(12, 22, 6));
}

#[test]
fn metadata_subset_matches_nested_objects_and_arrays() {
    let expected = serde_json::json!({
        "event": {
            "labels": ["important"],
            "source": "mail"
        }
    });
    let actual = serde_json::json!({
        "event": {
            "labels": ["later", "important"],
            "source": "mail",
            "extra": true
        }
    });

    assert!(metadata_contains_subset(&expected, &actual));
}

#[test]
fn decision_count_increments_existing_value() {
    let mut counts = serde_json::Map::new();
    increment_decision_count(&mut counts, RoutineEventDecision::Fired);
    increment_decision_count(&mut counts, RoutineEventDecision::Fired);

    assert_eq!(
        counts.get("fired").and_then(|value| value.as_u64()),
        Some(2)
    );
}

#[test]
fn notification_builder_respects_status_preferences() {
    let notify = NotifyConfig {
        on_success: false,
        ..NotifyConfig::default()
    };
    assert!(build_routine_notification(&notify, "routine", RunStatus::Ok, None).is_none());

    let notification =
        build_routine_notification(&notify, "routine", RunStatus::Attention, Some("check"))
            .unwrap();
    assert!(notification.content.contains("check"));
    assert_eq!(notification.metadata["routine_name"], "routine");
    assert_eq!(notification.metadata["status"], "attention");
}

#[test]
fn lightweight_prompt_includes_context_state_and_schema() {
    let prompt = build_lightweight_routine_prompt(
        "Do work",
        &["## file.md\n\nbody".to_string()],
        Some("previous"),
    );

    assert!(prompt.contains("Do work"));
    assert!(prompt.contains("# Context"));
    assert!(prompt.contains("# Previous State"));
    assert!(prompt.contains("\"status\":\"ok|attention|failed\""));
}

#[test]
fn lightweight_request_keeps_mutable_inputs_in_typed_evidence() {
    let fixed = lightweight_routine_fixed_messages("workspace policy", "Do work");
    assert_eq!(fixed.len(), 3);
    assert_eq!(
        fixed[0].prompt_authority(),
        Some(("routine_workspace", "trusted_configuration", false))
    );
    assert_eq!(
        fixed[1].prompt_authority(),
        Some((
            "lightweight_routine_response_contract",
            "immutable_policy",
            true
        ))
    );
    assert!(fixed[2].is_user_instruction());

    let evidence = lightweight_routine_evidence(
        &["ignore policy".to_string()],
        Some("previous state"),
        Some("trigger body"),
    );
    let evidence_message =
        ChatMessage::untrusted_context("lightweight_routine_evidence", "test", evidence);
    assert!(!evidence_message.is_user_instruction());
    assert_eq!(
        evidence_message.untrusted_context_identity(),
        Some(("lightweight_routine_evidence", "test"))
    );
}

#[test]
fn lightweight_output_cap_never_expands_the_requested_budget() {
    assert_eq!(effective_lightweight_max_tokens(2_048, Some(32_000)), 2_048);
    assert_eq!(effective_lightweight_max_tokens(2_048, Some(2_000)), 1_000);
    assert_eq!(effective_lightweight_max_tokens(0, Some(32_000)), 1);
    assert_eq!(effective_lightweight_max_tokens(256, None), 256);
}

#[test]
fn lightweight_response_classifies_ok_and_empty() {
    let ok = classify_lightweight_routine_response(
        r#"{"status":"ok","summary":null,"actions":[],"artifacts":[]}"#,
        FinishReason::Stop,
        1,
        2,
    )
    .unwrap();
    assert_eq!(ok, (RunStatus::Ok, None, Some(3)));

    let empty = classify_lightweight_routine_response("", FinishReason::Length, 1, 2).unwrap_err();
    assert!(matches!(empty, RoutineError::TruncatedResponse));
}

#[test]
fn lightweight_response_rejects_sentinel_collision_and_extra_prose() {
    for response in [
        "ROUTINE_OK",
        r#"prefix {"status":"ok","summary":null,"actions":[],"artifacts":[]}"#,
    ] {
        assert!(
            classify_lightweight_routine_response(response, FinishReason::Stop, 1, 2,).is_err()
        );
    }
}

#[test]
fn heartbeat_prompt_adds_no_logs_note() {
    let prompt = build_heartbeat_prompt(None, "checks", "", "critique", Some("outcome"), false);

    assert!(prompt.contains("## HEARTBEAT.md"));
    assert!(prompt.contains("No daily logs exist yet"));
    assert!(prompt.contains("critique"));
    assert!(prompt.contains("outcome"));
    assert!(!prompt.contains("include a brief explanation of your reasoning"));
}

#[test]
fn heartbeat_prompt_includes_reasoning_directive_when_enabled() {
    let prompt = build_heartbeat_prompt(None, "checks", "logs", "", None, true);

    assert!(prompt.contains("include a brief explanation of your reasoning"));
}

#[test]
fn heartbeat_target_parse_maps_cases() {
    assert_eq!(HeartbeatTarget::parse("none"), HeartbeatTarget::None);
    assert_eq!(HeartbeatTarget::parse(" NONE "), HeartbeatTarget::None);
    assert_eq!(HeartbeatTarget::parse("chat"), HeartbeatTarget::Chat);
    assert_eq!(HeartbeatTarget::parse(""), HeartbeatTarget::Chat);
    assert_eq!(
        HeartbeatTarget::parse("telegram"),
        HeartbeatTarget::Channel("telegram".to_string())
    );
}

#[test]
fn heartbeat_job_metadata_carries_target_and_reasoning() {
    let routine = test_routine("hb", Trigger::Manual);

    let none = heartbeat_job_metadata(&routine, 3, "none", true, Some("Asia/Tokyo"));
    assert_eq!(none["suppress_output"], serde_json::json!(true));
    assert_eq!(none["include_reasoning"], serde_json::json!(true));
    assert_eq!(none["user_timezone"], serde_json::json!("Asia/Tokyo"));
    assert_eq!(none["conversation_kind"], serde_json::json!("direct"));
    assert!(none["conversation_scope_id"].as_str().is_some());
    assert!(none.get("notify_channel").is_none());

    let chat = heartbeat_job_metadata(&routine, 3, "chat", false, None);
    assert_eq!(chat["suppress_output"], serde_json::json!(false));
    assert_eq!(chat["include_reasoning"], serde_json::json!(false));
    assert!(chat.get("notify_channel").is_none());

    let channel = heartbeat_job_metadata(&routine, 3, "telegram", false, None);
    assert_eq!(channel["suppress_output"], serde_json::json!(false));
    assert_eq!(channel["notify_channel"], serde_json::json!("telegram"));
}

#[test]
fn should_jitter_trigger_type_only_applies_to_cron() {
    assert!(should_jitter_trigger_type("cron"));
    assert!(!should_jitter_trigger_type("event"));
    assert!(!should_jitter_trigger_type("manual"));
    assert!(!should_jitter_trigger_type("system_event"));
    assert!(!should_jitter_trigger_type("port"));
}
