use super::*;

fn value_for<'a>(plan: &'a SetupBootstrapEnvPlan, key: &str) -> Option<&'a str> {
    plan.variables()
        .iter()
        .find(|var| var.key == key)
        .map(|var| var.value.as_str())
}

#[test]
fn default_input_has_empty_plan() {
    let plan = setup_bootstrap_env_plan(&SetupBootstrapEnvInput::default());

    assert!(plan.is_empty());
}

#[test]
fn runtime_profile_writes_headless_markers_after_onboard_marker() {
    let input = SetupBootstrapEnvInput {
        onboard_completed: true,
        runtime_profile: Some(SetupRuntimeProfile::PiOsLite64),
        ..SetupBootstrapEnvInput::default()
    };

    let plan = setup_bootstrap_env_plan(&input);

    assert_eq!(value_for(&plan, "ONBOARD_COMPLETED"), Some("true"));
    assert_eq!(
        value_for(&plan, "THINCLAW_RUNTIME_PROFILE"),
        Some("pi-os-lite-64")
    );
    assert_eq!(value_for(&plan, "THINCLAW_HEADLESS"), Some("true"));
    let keys: Vec<&str> = plan.variables().iter().map(|var| var.key).collect();
    assert_eq!(
        keys,
        vec![
            "ONBOARD_COMPLETED",
            "THINCLAW_RUNTIME_PROFILE",
            "THINCLAW_HEADLESS"
        ]
    );
}

#[test]
fn channel_mapping_preserves_existing_enabled_and_false_values() {
    let input = SetupBootstrapEnvInput {
        channels: SetupBootstrapChannelInput {
            signal_allow_from_groups: Some(String::new()),
            signal_group_allow_from: Some("group-a".to_string()),
            http_enabled: true,
            http_host: Some("0.0.0.0".to_string()),
            http_port: Some(8080),
            apple_mail_unread_only: false,
            apple_mail_mark_as_read: false,
            gateway_enabled: Some(false),
            cli_enabled: Some(false),
            ..SetupBootstrapChannelInput::default()
        },
        web_ui: SetupBootstrapWebUiInput {
            show_branding: false,
            ..SetupBootstrapWebUiInput::default()
        },
        ..SetupBootstrapEnvInput::default()
    };

    let plan = setup_bootstrap_env_plan(&input);

    assert_eq!(value_for(&plan, "HTTP_ENABLED"), Some("true"));
    assert_eq!(value_for(&plan, "HTTP_HOST"), Some("0.0.0.0"));
    assert_eq!(value_for(&plan, "HTTP_PORT"), Some("8080"));
    assert_eq!(value_for(&plan, "SIGNAL_ALLOW_FROM_GROUPS"), None);
    assert_eq!(value_for(&plan, "SIGNAL_GROUP_ALLOW_FROM"), Some("group-a"));
    assert_eq!(value_for(&plan, "APPLE_MAIL_UNREAD_ONLY"), Some("false"));
    assert_eq!(value_for(&plan, "APPLE_MAIL_MARK_AS_READ"), Some("false"));
    assert_eq!(value_for(&plan, "GATEWAY_ENABLED"), Some("false"));
    assert_eq!(value_for(&plan, "CLI_ENABLED"), Some("false"));
    assert_eq!(value_for(&plan, "WEBCHAT_SHOW_BRANDING"), Some("false"));
    assert_eq!(value_for(&plan, "WEBCHAT_THEME"), None);
}

#[test]
fn quick_setup_plan_uses_all_ten_target_sections() {
    let plan = setup_wizard_plan(SetupWizardPlanInput::default());

    assert_eq!(plan.phases.len(), 10);
    assert_eq!(plan.steps.len(), 19);
    assert!(
        !plan
            .steps
            .iter()
            .any(|step| step.id == SetupWizardStepId::SmartRouting)
    );
    assert!(
        plan.steps
            .iter()
            .any(|step| step.id == SetupWizardStepId::CodingWorkers)
    );
    assert_eq!(
        plan.phases.iter().map(|phase| phase.id).collect::<Vec<_>>(),
        ALL_SETUP_WIZARD_PHASE_IDS
    );
    assert!(!plan.steps.iter().any(|step| !step.id.executable()));
    assert_eq!(
        plan.phase(SetupWizardPhaseId::Profile)
            .map(|phase| phase.step_ids.as_slice()),
        Some([SetupWizardStepId::Profile].as_slice())
    );
}

#[test]
fn guided_and_channels_only_plans_keep_expected_shape() {
    let ai_plan = setup_wizard_plan(SetupWizardPlanInput {
        channels_only: false,
        guide_topic: Some(SetupGuideTopic::Ai),
        mode: SetupMode::Advanced,
    });
    let ai_step_ids: Vec<_> = ai_plan.steps.iter().map(|step| step.id).collect();
    assert_eq!(
        ai_step_ids,
        vec![
            SetupWizardStepId::InferenceProvider,
            SetupWizardStepId::ModelSelection,
            SetupWizardStepId::SmartRouting,
            SetupWizardStepId::FallbackProviders,
            SetupWizardStepId::Embeddings,
            SetupWizardStepId::ChannelVerification,
            SetupWizardStepId::Summary,
        ]
    );

    let channels_plan = setup_wizard_plan(SetupWizardPlanInput {
        channels_only: true,
        guide_topic: Some(SetupGuideTopic::Ai),
        mode: SetupMode::Advanced,
    });
    let channel_step_ids: Vec<_> = channels_plan.steps.iter().map(|step| step.id).collect();
    assert_eq!(
        channel_step_ids,
        vec![
            SetupWizardStepId::Channels,
            SetupWizardStepId::ChannelVerification,
            SetupWizardStepId::Summary,
        ]
    );
}

#[test]
fn advanced_plan_traverses_every_executable_legacy_page_once() {
    let plan = setup_wizard_plan(SetupWizardPlanInput {
        mode: SetupMode::Advanced,
        ..SetupWizardPlanInput::default()
    });
    let ids = plan
        .phases
        .iter()
        .flat_map(|phase| phase.step_ids.iter().copied())
        .collect::<Vec<_>>();
    let unique = ids.iter().copied().collect::<BTreeSet<_>>();
    let executable_count = ALL_SETUP_WIZARD_STEP_IDS
        .iter()
        .filter(|step| step.executable())
        .count();
    assert_eq!(ids.len(), executable_count);
    assert_eq!(unique.len(), executable_count);
    assert_eq!(plan.steps.len(), executable_count);
    assert!(!unique.contains(&SetupWizardStepId::ChannelContinuity));
}

#[test]
fn setup_plan_digest_binds_secret_free_review_content() {
    assert!(SetupSettingChange::new("provider.api_key", None, None).is_err());
    let setting = SetupSettingChange::new(
        "runtime.profile",
        None,
        Some(SetupSettingValue::Text("remote".to_string())),
    )
    .unwrap();
    let plan = SetupPlan {
        schema_version: 1,
        baseline_revision: "baseline-1".to_string(),
        digest: String::new(),
        mode: SetupMode::Advanced,
        profile: "remote".to_string(),
        settings_diff: vec![setting],
        actions: vec![SetupAction::MarkSetupCompleted],
        warnings: Vec::new(),
        blockers: Vec::new(),
        continuation: SetupContinuation::Exit,
    }
    .seal()
    .unwrap();
    assert!(plan.digest_matches(&plan.digest));

    let mut changed = plan.clone();
    changed.profile = "balanced".to_string();
    assert!(!changed.digest_matches(&plan.digest));
}

#[test]
fn provider_planning_uses_catalog_and_legacy_fallbacks() {
    assert_eq!(provider_display_name("openai"), "OpenAI");
    assert_eq!(provider_default_model("openai").as_deref(), Some("gpt-4o"));
    assert_eq!(
        suggested_cheap_model_for_provider("openai", Some("gpt-4o")).as_deref(),
        Some("gpt-4o-mini")
    );
    assert_eq!(
        suggested_cheap_model_for_provider("openai", Some("gpt-4o-mini")),
        provider_default_model("openai")
    );

    assert_eq!(provider_display_name("llama_cpp"), "llama.cpp");
    assert_eq!(
        provider_default_model("llama_cpp").as_deref(),
        Some("llama-local")
    );
    assert_eq!(provider_display_name("custom_provider"), "custom_provider");
    assert_eq!(provider_default_model("custom_provider"), None);
}

#[test]
fn quick_embeddings_defaults_follow_primary_provider_class() {
    let remote = setup_quick_embeddings_defaults(Some("openai"));
    assert!(remote.enabled);
    assert_eq!(remote.provider, "openai");
    assert_eq!(remote.model, "text-embedding-3-small");

    let local = setup_quick_embeddings_defaults(Some("llama_cpp"));
    assert!(local.enabled);
    assert_eq!(local.provider, "ollama");
    assert_eq!(local.model, "nomic-embed-text");
}

#[test]
fn provider_slot_defaults_plan_fills_missing_slots_without_overwriting_existing() {
    let empty = setup_provider_slot_defaults(&SetupProviderSlotDefaultsInput {
        provider_slug: "openai".to_string(),
        ..SetupProviderSlotDefaultsInput::default()
    });
    assert_eq!(empty.primary.as_deref(), Some("gpt-4o"));
    assert_eq!(empty.cheap.as_deref(), Some("gpt-4o-mini"));

    let from_current_primary = setup_provider_slot_defaults(&SetupProviderSlotDefaultsInput {
        provider_slug: "openai_compatible".to_string(),
        current_primary_model: Some("primary-from-settings".to_string()),
        existing_primary: None,
        existing_cheap: None,
    });
    assert_eq!(
        from_current_primary.primary.as_deref(),
        Some("primary-from-settings")
    );
    assert_eq!(
        from_current_primary.cheap.as_deref(),
        Some("default"),
        "fallback cheap should prefer provider default when distinct from current primary"
    );

    let existing = setup_provider_slot_defaults(&SetupProviderSlotDefaultsInput {
        provider_slug: "openai".to_string(),
        current_primary_model: Some("ignored-current".to_string()),
        existing_primary: Some("kept-primary".to_string()),
        existing_cheap: Some("kept-cheap".to_string()),
    });
    assert_eq!(existing.primary.as_deref(), Some("kept-primary"));
    assert_eq!(existing.cheap.as_deref(), Some("kept-cheap"));
}

#[test]
fn profile_metadata_maps_headless_runtime_profiles() {
    assert_eq!(
        SetupOnboardingProfile::CustomAdvanced.title(),
        "Custom / Advanced"
    );
    assert!(
        SetupOnboardingProfile::CustomAdvanced
            .description()
            .contains("neutral baseline")
    );
    assert_eq!(
        SetupOnboardingProfile::RemoteServer.runtime_profile(),
        Some(SetupRuntimeProfile::Remote)
    );
    assert_eq!(
        SetupOnboardingProfile::PiOsLite64.runtime_profile_env_value(),
        Some("pi-os-lite-64")
    );
    assert!(SetupOnboardingProfile::RemoteServer.is_headless_remote());
    assert!(!SetupOnboardingProfile::Balanced.is_headless_remote());
}

#[test]
fn runtime_command_policy_distinguishes_desktop_and_headless_profiles() {
    let desktop = SetupRuntimeCommandInput {
        profile: SetupOnboardingProfile::Balanced,
        ui_mode: SetupWizardUiMode::Tui,
        continuation: SetupContinuation::Tui,
    };
    assert_eq!(setup_primary_runtime_command(&desktop), "thinclaw tui");
    assert!(
        setup_runtime_handoff_summary(&desktop).contains("`thinclaw tui`"),
        "desktop handoff should name the selected UI command"
    );
    assert!(
        !setup_what_next_commands(&desktop)
            .iter()
            .any(|command| command == "Topic guide: thinclaw setup edit")
    );

    let headless = SetupRuntimeCommandInput {
        profile: SetupOnboardingProfile::PiOsLite64,
        ui_mode: SetupWizardUiMode::Tui,
        continuation: SetupContinuation::Exit,
    };
    assert_eq!(
        setup_primary_runtime_command(&headless),
        "thinclaw run --skip-setup-check"
    );
    assert!(
        setup_runtime_handoff_summary(&headless).contains("install/start the OS service"),
        "paused headless handoff should point at service startup"
    );
    assert!(
        setup_what_next_commands(&headless)
            .iter()
            .any(|command| command
                == "Pi diagnostics: thinclaw doctor --readiness-profile pi-os-lite-64")
    );
}
