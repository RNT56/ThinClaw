use std::path::PathBuf;

use crate::*;
use chrono::Utc;
use serde::Deserialize;
use thinclaw_history::OutcomeContract;
use uuid::Uuid;

#[test]
fn merge_json_recursively_overlays_objects() {
    let base = serde_json::json!({
        "a": 1,
        "nested": {
            "left": true,
            "replace": "old"
        }
    });
    let overlay = serde_json::json!({
        "nested": {
            "replace": "new",
            "right": 2
        }
    });

    let merged = merge_json(&base, &overlay);
    assert_eq!(merged["a"], 1);
    assert_eq!(merged["nested"]["left"], true);
    assert_eq!(merged["nested"]["replace"], "new");
    assert_eq!(merged["nested"]["right"], 2);
}

#[test]
fn target_signature_normalizes_link_identity() {
    let metadata = serde_json::json!({
        "provider": "OpenAI",
        "model": "GPT-5",
        "route_key": "Primary",
        "asset_id": "Prompt/User"
    });

    assert_eq!(
        target_signature(ExperimentTargetKind::PromptAsset, &metadata).as_deref(),
        Some("PromptAsset|openai|gpt-5|primary|prompt/user")
    );
}

#[test]
fn ensure_unique_target_signature_detects_duplicates_and_honors_skip() {
    let id = Uuid::new_v4();
    let metadata = serde_json::json!({
        "provider": "openai",
        "model": "gpt-5"
    });
    let targets = vec![ExperimentTarget {
        id,
        name: "existing".to_string(),
        kind: ExperimentTargetKind::InferenceConfig,
        location: None,
        metadata: metadata.clone(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }];

    assert!(
        ensure_unique_target_signature(
            ExperimentTargetKind::InferenceConfig,
            &metadata,
            None,
            &targets
        )
        .is_err()
    );
    assert!(
        ensure_unique_target_signature(
            ExperimentTargetKind::InferenceConfig,
            &metadata,
            Some(id),
            &targets
        )
        .is_ok()
    );
}

#[test]
fn derive_opportunities_groups_usage_and_links_targets() {
    let now = Utc::now();
    let target_id = Uuid::new_v4();
    let usage = vec![ExperimentModelUsageRecord {
        id: Uuid::new_v4(),
        provider: "OpenAI".to_string(),
        model: "gpt-5".to_string(),
        route_key: Some("primary".to_string()),
        logical_role: Some("planner".to_string()),
        endpoint_type: Some("hosted".to_string()),
        workload_tag: Some("json_parser".to_string()),
        latency_ms: Some(120),
        cost_usd: Some(0.01),
        success: false,
        prompt_asset_ids: vec!["system".to_string()],
        retrieval_asset_ids: Vec::new(),
        tool_policy_ids: Vec::new(),
        evaluator_ids: Vec::new(),
        parser_ids: Vec::new(),
        metadata: serde_json::json!({}),
        created_at: now,
    }];
    let targets = vec![ExperimentTarget {
        id: target_id,
        name: "prompt".to_string(),
        kind: ExperimentTargetKind::PromptAsset,
        location: None,
        metadata: serde_json::json!({
            "provider": "openai",
            "model": "gpt-5",
            "route_key": "primary",
            "asset_id": "system",
        }),
        created_at: now,
        updated_at: now,
    }];
    let links = vec![ExperimentTargetLink {
        id: Uuid::new_v4(),
        target_id,
        kind: ExperimentTargetKind::PromptAsset,
        provider: "openai".to_string(),
        model: "gpt-5".to_string(),
        route_key: Some("primary".to_string()),
        logical_role: Some("planner".to_string()),
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }];

    let mut opportunities = derive_opportunities(&usage, &targets, &links);
    sort_experiment_opportunities(&mut opportunities);

    assert!(
        opportunities
            .iter()
            .any(
                |opportunity| opportunity.opportunity_type == ExperimentTargetKind::PromptAsset
                    && opportunity.linked_target_id == Some(target_id)
                    && opportunity.metadata["call_count"] == 1
                    && opportunity.metadata["error_count"] == 1
            )
    );
    assert!(
        opportunities
            .iter()
            .any(|opportunity| opportunity.opportunity_type == ExperimentTargetKind::Parser)
    );
}

#[test]
fn derive_outcome_opportunities_uses_negative_contract_patterns() {
    let now = Utc::now();
    let contract = OutcomeContract {
        id: Uuid::new_v4(),
        user_id: "user".to_string(),
        actor_id: None,
        channel: None,
        thread_id: None,
        source_kind: "turn".to_string(),
        source_id: "turn-1".to_string(),
        contract_type: "turn_usefulness".to_string(),
        status: "evaluated".to_string(),
        summary: None,
        due_at: now,
        expires_at: now,
        final_verdict: Some("negative".to_string()),
        final_score: Some(0.1),
        evaluation_details: serde_json::json!({}),
        metadata: serde_json::json!({
            "pattern_key": "prompt-drift"
        }),
        dedupe_key: "dedupe".to_string(),
        claimed_at: None,
        evaluated_at: Some(now),
        created_at: now,
        updated_at: now,
    };
    let target_id = Uuid::new_v4();
    let target = ExperimentTarget {
        id: target_id,
        name: "USER.md".to_string(),
        kind: ExperimentTargetKind::PromptAsset,
        location: None,
        metadata: serde_json::json!({
            "asset_id": "USER.md",
            "pattern_key": "prompt-drift"
        }),
        created_at: now,
        updated_at: now,
    };

    let opportunities = derive_outcome_opportunities(&[contract], &[target], 10, "USER.md");

    assert_eq!(opportunities.len(), 1);
    assert_eq!(
        opportunities[0].opportunity_type,
        ExperimentTargetKind::PromptAsset
    );
    assert_eq!(opportunities[0].linked_target_id, Some(target_id));
    assert_eq!(opportunities[0].source.as_deref(), Some("outcome_learning"));
    assert!(opportunities[0].summary.contains("USER.md"));
}

#[test]
fn validate_project_workdir_fragment_rejects_parent_traversal() {
    let error = validate_project_workdir_fragment("../escape")
        .expect_err("parent traversal should be rejected");
    assert!(error.contains("Project workdir must stay inside the workspace root"));
}

#[test]
fn normalize_trial_completion_adds_default_stage() {
    let completion = ExperimentRunnerCompletion {
        exit_code: Some(0),
        metrics_json: serde_json::json!({}),
        summary: Some("ok".to_string()),
        runtime_ms: Some(42),
        attributed_cost_usd: None,
        log_preview_path: None,
        artifact_manifest_json: serde_json::Value::Null,
    };

    let normalized = normalize_trial_completion(completion);
    assert_eq!(
        normalized
            .artifact_manifest_json
            .get("stage")
            .and_then(|value| value.as_str()),
        Some("complete")
    );
}

#[test]
fn changed_path_policy_filters_internal_paths_and_enforces_allowlist() {
    let changed = filtered_changed_files(vec![
        ".thinclaw-experiments/state.json".to_string(),
        "src/lib.rs".to_string(),
        "README.md".to_string(),
    ]);

    assert_eq!(
        changed,
        vec!["src/lib.rs".to_string(), "README.md".to_string()]
    );
    assert!(enforce_mutable_paths(&["src".to_string()], &changed).is_err());
    assert!(enforce_mutable_paths(&["src".to_string(), "README.md".to_string()], &changed).is_ok());
}

#[test]
fn env_pairs_from_json_keeps_only_string_values() {
    let mut pairs = env_pairs_from_json(&serde_json::json!({
        "TOKEN": "secret",
        "COUNT": 3,
        "EMPTY": ""
    }));
    pairs.sort();

    assert_eq!(
        pairs,
        vec![
            ("EMPTY".to_string(), "".to_string()),
            ("TOKEN".to_string(), "secret".to_string())
        ]
    );
}

#[test]
fn parse_secret_reference_infers_uppercase_env_alias() {
    assert_eq!(
        parse_secret_reference("runpod_api_key"),
        Some((
            "runpod_api_key".to_string(),
            vec!["runpod_api_key".to_string(), "RUNPOD_API_KEY".to_string()]
        ))
    );
    assert_eq!(
        parse_secret_reference("runpod:RUNPOD_API_KEY"),
        Some(("runpod".to_string(), vec!["RUNPOD_API_KEY".to_string()]))
    );
}

#[test]
fn ready_project_status_requires_workspace_mutable_paths_and_command() {
    let now = Utc::now();
    let mut project = ExperimentProject {
        id: Uuid::new_v4(),
        name: "demo".to_string(),
        workspace_path: ".".to_string(),
        git_remote_name: "origin".to_string(),
        base_branch: "main".to_string(),
        preset: Default::default(),
        strategy_prompt: "test".to_string(),
        workdir: ".".to_string(),
        prepare_command: None,
        run_command: "echo ok".to_string(),
        mutable_paths: vec!["src".to_string()],
        fixed_paths: Vec::new(),
        primary_metric: ExperimentMetricDefinition::default(),
        secondary_metrics: Vec::new(),
        comparison_policy: Default::default(),
        stop_policy: Default::default(),
        default_runner_profile_id: None,
        promotion_mode: "manual".to_string(),
        autonomy_mode: Default::default(),
        status: ExperimentProjectStatus::Draft,
        created_at: now,
        updated_at: now,
    };

    assert_eq!(
        ready_project_status(&project, true),
        ExperimentProjectStatus::Ready
    );
    assert_eq!(
        ready_project_status(&project, false),
        ExperimentProjectStatus::Draft
    );
    project.mutable_paths.clear();
    assert_eq!(
        ready_project_status(&project, true),
        ExperimentProjectStatus::Draft
    );
    project.mutable_paths.push("src".to_string());
    project.run_command = "   ".to_string();
    assert_eq!(
        ready_project_status(&project, true),
        ExperimentProjectStatus::Draft
    );
}

#[test]
fn recent_trial_context_renders_latest_trial_summary() {
    let now = Utc::now();
    let trial = ExperimentTrial {
        id: Uuid::new_v4(),
        campaign_id: Uuid::new_v4(),
        sequence: 7,
        candidate_commit: None,
        parent_best_commit: None,
        status: ExperimentTrialStatus::Accepted,
        runner_backend: ExperimentRunnerBackend::LocalDocker,
        exit_code: Some(0),
        metrics_json: serde_json::json!({ "score": 0.95 }),
        summary: Some("candidate improved score".to_string()),
        decision_reason: None,
        log_preview_path: None,
        artifact_manifest_json: serde_json::json!({}),
        runtime_ms: Some(100),
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: Some("tune parser".to_string()),
        mutation_summary: None,
        reviewer_decision: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: Some(now),
        completed_at: Some(now),
        created_at: now,
        updated_at: now,
    };

    let context = recent_trial_context(&[trial]);

    assert!(context.contains("Trial #7"));
    assert!(context.contains("status=Accepted"));
    assert!(context.contains("hypothesis=tune parser"));
}

#[test]
fn truncate_for_prompt_preserves_short_text_and_truncates_long_text() {
    assert_eq!(truncate_for_prompt("short", 10), "short");
    assert_eq!(truncate_for_prompt("abcdef", 5), "ab...");
}

#[test]
fn parse_research_json_response_accepts_fenced_json() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Response {
        ok: bool,
    }

    let parsed: Response =
        parse_research_json_response("```json\n{\"ok\":true}\n```").expect("fenced json");
    assert_eq!(parsed, Response { ok: true });
    assert!(parse_research_json_response::<Response>("not json").is_err());
}

#[test]
fn lease_completion_rejection_message_covers_terminal_statuses() {
    assert_eq!(
        lease_completion_rejection_message(ExperimentLeaseStatus::Completed),
        "lease completion was already recorded; repeated terminal completions are ignored"
    );
    assert_eq!(
        lease_completion_rejection_message(ExperimentLeaseStatus::Revoked),
        "lease was revoked before completion and can no longer transition to terminal"
    );
}

#[test]
fn lease_runner_trial_status_maps_runner_progress_strings() {
    for status in ["runner_started", "running_prepare", "running_benchmark"] {
        assert_eq!(
            lease_runner_trial_status(status, ExperimentTrialStatus::Preparing),
            ExperimentTrialStatus::Running
        );
    }
    for status in ["evaluating", "uploading_artifacts", "completing"] {
        assert_eq!(
            lease_runner_trial_status(status, ExperimentTrialStatus::Running),
            ExperimentTrialStatus::Evaluating
        );
    }
}

#[test]
fn lease_runner_trial_status_preserves_unknown_statuses() {
    assert_eq!(
        lease_runner_trial_status("runner_started ", ExperimentTrialStatus::Preparing),
        ExperimentTrialStatus::Preparing
    );
    assert_eq!(
        lease_runner_trial_status("custom_status", ExperimentTrialStatus::Accepted),
        ExperimentTrialStatus::Accepted
    );
}

#[test]
fn validate_lease_completion_status_requires_claimed_lease() {
    assert_eq!(
        validate_lease_completion_status(ExperimentLeaseStatus::Claimed),
        Ok(())
    );
    assert_eq!(
        validate_lease_completion_status(ExperimentLeaseStatus::Pending),
        Err("lease must be claimed before completion can be recorded")
    );
    assert_eq!(
        validate_lease_completion_status(ExperimentLeaseStatus::Completed),
        Err("lease completion was already recorded; repeated terminal completions are ignored")
    );
}

#[test]
fn experiment_api_messages_preserve_existing_text() {
    let id = Uuid::from_u128(7);

    assert_eq!(
        experiment_project_not_found_message(id),
        format!("Experiment project {id} not found")
    );
    assert_eq!(
        experiments_feature_disabled_message(),
        "Enable experiments in Settings → Features to use this API."
    );
    assert_eq!(
        experiment_workspace_path_missing_message("/tmp/project"),
        "Workspace path does not exist: /tmp/project"
    );
    assert_eq!(
        experiment_workspace_path_missing_with_error_message("/tmp/project", "missing"),
        "Workspace path does not exist: /tmp/project (missing)"
    );
    assert_eq!(
        experiment_project_workdir_missing_message("/tmp/project/bench", "missing"),
        "Project workdir does not exist: /tmp/project/bench (missing)"
    );
    assert_eq!(
        experiment_project_workdir_outside_workspace_message(),
        "Project workdir resolves outside the workspace root."
    );
    assert_eq!(
        experiment_project_missing_mutable_paths_message(),
        "Project must define at least one mutable path before launch."
    );
    assert_eq!(
        experiment_project_run_command_empty_message(),
        "Project run_command must not be empty."
    );
    assert_eq!(
        experiment_workspace_not_git_repository_message("fatal"),
        "Workspace path is not a git repository ThinClaw can use: fatal"
    );
    assert_eq!(
        experiment_project_workdir_escapes_campaign_worktree_message(),
        "Project workdir escapes the campaign worktree."
    );
    assert_eq!(
        experiment_runner_not_found_message(id),
        format!("Experiment runner {id} not found")
    );
    assert_eq!(
        experiment_campaign_not_found_message(id),
        format!("Experiment campaign {id} not found")
    );
    assert_eq!(
        experiment_trial_not_found_message(id),
        format!("Experiment trial {id} not found")
    );
    assert_eq!(
        experiment_target_not_found_message(id),
        format!("Experiment target {id} not found")
    );
    assert_eq!(
        experiment_opportunity_not_found_message(id),
        format!("Experiment opportunity {id} not found")
    );
    assert_eq!(
        experiment_lease_not_found_message(id),
        format!("Experiment lease {id} not found")
    );
    assert_eq!(
        experiment_base_branch_unavailable_message("main", "missing"),
        "Base branch 'main' is not available locally: missing"
    );
    assert_eq!(
        experiment_git_remote_unavailable_message("origin", "missing"),
        "Configured git remote 'origin' is not available: missing"
    );
    assert_eq!(
        experiment_campaign_has_no_worktree_message(),
        "Campaign has no worktree"
    );
    assert_eq!(
        experiment_campaign_has_no_trial_to_reissue_message(),
        "Campaign has no trial to reissue a lease for."
    );
    assert_eq!(
        experiment_campaign_has_no_accepted_commit_message(),
        "Campaign has no accepted commit to promote"
    );
    assert_eq!(
        experiment_promotion_pr_body(id, "abc123", "latency"),
        format!(
            "Promoting best commit from experiment campaign {id}\n\nBest commit: abc123\nPrimary metric: latency"
        )
    );
    assert_eq!(
        experiment_primary_metric_not_found_message("latency"),
        "Primary metric 'latency' was not found in the runner result."
    );
    assert_eq!(
        research_subagent_executor_unavailable_message(),
        "Research subagent executor is not available."
    );
    assert_eq!(
        experiment_campaign_missing_worktree_path_message(),
        "Campaign missing worktree path"
    );
    assert_eq!(
        experiment_campaign_missing_worktree_path_field_message(),
        "Campaign missing worktree_path"
    );
    assert_eq!(
        experiment_campaign_missing_experiment_branch_field_message(),
        "Campaign missing experiment_branch"
    );
    assert_eq!(
        experiment_campaign_missing_experiment_branch_message(),
        "Campaign missing experiment branch"
    );
    assert_eq!(
        experiment_no_candidate_changes_message(),
        "No candidate changes detected in the campaign worktree."
    );
    assert_eq!(experiment_lease_revoked_action_message(), "Lease revoked.");
    assert_eq!(
        experiment_campaign_paused_by_operator_message(),
        "Paused by operator."
    );
    assert_eq!(experiment_campaign_paused_message(), "Campaign paused.");
    assert_eq!(
        experiment_campaign_cancelled_by_operator_message(),
        "Cancelled by operator."
    );
    assert_eq!(
        experiment_campaign_cancelled_message(),
        "Campaign cancelled."
    );
    assert_eq!(
        experiment_lease_reissue_remote_only_message(),
        "Lease reissue is only supported for remote runners."
    );
    assert_eq!(
        experiment_remote_trial_reissue_in_flight_only_message(),
        "Only in-flight remote trials can receive a new lease."
    );
    assert_eq!(
        experiment_target_id_required_message(),
        "target_id is required"
    );
    assert_eq!(
        experiment_runner_profile_id_required_message(),
        "runner_profile_id is required"
    );
    assert_eq!(experiment_lease_revoked_message(), "Lease has been revoked");
    assert_eq!(experiment_lease_expired_message(), "Lease has expired");
    assert_eq!(
        invalid_experiment_lease_token_message(),
        "Invalid lease token"
    );
}

#[test]
fn campaign_path_helpers_build_stable_short_paths_and_gateway_url() {
    let now = Utc::now();
    let campaign_id = Uuid::parse_str("12345678-90ab-cdef-1234-567890abcdef").unwrap();
    let campaign = ExperimentCampaign {
        id: campaign_id,
        project_id: Uuid::new_v4(),
        runner_profile_id: Uuid::new_v4(),
        owner_user_id: "user".to_string(),
        status: ExperimentCampaignStatus::Running,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: None,
        remote_ref: None,
        worktree_path: None,
        started_at: Some(now),
        ended_at: None,
        trial_count: 0,
        failure_count: 0,
        pause_reason: None,
        queue_state: ExperimentCampaignQueueState::Active,
        queue_position: 0,
        active_trial_id: None,
        total_runtime_ms: 0,
        total_cost_usd: 0.0,
        total_llm_cost_usd: 0.0,
        total_runner_cost_usd: 0.0,
        consecutive_non_improving_trials: 0,
        max_trials_override: None,
        gateway_url: Some(" https://example.test/gateway ".to_string()),
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };

    assert_eq!(short_id(campaign_id), "1234567890ab");
    assert_eq!(
        experiments_worktree_path("/workspace", campaign_id),
        PathBuf::from("/workspace/.thinclaw-experiments/1234567890ab")
    );
    assert_eq!(
        campaign_gateway_url(&campaign).as_deref(),
        Some("https://example.test/gateway")
    );
}

#[test]
fn runpod_cost_is_normalized_from_credits() {
    let (usd_per_hour, source, native_hourly_rate, native_currency, normalization) =
        provider_hourly_rate_usd(
            &serde_json::json!({
                "pod": {
                    "adjustedCostPerHr": 1.75
                }
            }),
            ExperimentRunnerBackend::Runpod,
        )
        .expect("runpod metadata should produce a cost");
    assert!((usd_per_hour - 1.75).abs() < 1e-9);
    assert_eq!(source, "pod.adjustedCostPerHr");
    assert_eq!(native_hourly_rate, Some(1.75));
    assert_eq!(native_currency.as_deref(), Some("runpod_credits"));
    assert_eq!(
        normalization.as_deref(),
        Some("assumed_1_credit_equals_1_usd")
    );
}

#[test]
fn llm_usage_summary_groups_costs_by_role_and_provider() {
    let records = vec![
        ExperimentModelUsageRecord {
            id: Uuid::new_v4(),
            provider: "openai".to_string(),
            model: "gpt-5.4-mini".to_string(),
            route_key: Some("planner|openai|gpt-5.4-mini".to_string()),
            logical_role: Some("planner".to_string()),
            endpoint_type: None,
            workload_tag: None,
            latency_ms: Some(100),
            cost_usd: Some(0.12),
            success: true,
            prompt_asset_ids: Vec::new(),
            retrieval_asset_ids: Vec::new(),
            tool_policy_ids: Vec::new(),
            evaluator_ids: Vec::new(),
            parser_ids: Vec::new(),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        },
        ExperimentModelUsageRecord {
            id: Uuid::new_v4(),
            provider: "openai".to_string(),
            model: "gpt-5.4-mini".to_string(),
            route_key: Some("mutator|openai|gpt-5.4-mini".to_string()),
            logical_role: Some("mutator".to_string()),
            endpoint_type: None,
            workload_tag: None,
            latency_ms: Some(200),
            cost_usd: Some(0.08),
            success: true,
            prompt_asset_ids: Vec::new(),
            retrieval_asset_ids: Vec::new(),
            tool_policy_ids: Vec::new(),
            evaluator_ids: Vec::new(),
            parser_ids: Vec::new(),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        },
    ];
    let summary = summarize_llm_usage(&records, "trial_id");
    assert!((summary.total_usd - 0.20).abs() < 1e-9);
    assert_eq!(summary.details["source"], "trial_id");
    assert_eq!(summary.details["usage_record_count"], 2);
    assert_eq!(summary.details["by_role_usd"]["planner"], 0.12);
    assert_eq!(summary.details["by_role_usd"]["mutator"], 0.08);
    assert_eq!(summary.details["by_provider_usd"]["openai"], 0.20);
}
