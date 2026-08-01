use super::*;

#[cfg(any(feature = "postgres", feature = "libsql"))]
#[test]
fn test_cli_guide_onboarding_keeps_runtime_handoff_enabled() {
    let config = setup_config_for_onboard_command(
        false,
        false,
        Some(thinclaw::setup::GuideTopic::Menu),
        UiMode::Cli,
        None,
    );

    assert_eq!(config.guide_topic, Some(thinclaw::setup::GuideTopic::Menu));
    assert!(config.invocation.continuation.continues_to_runtime());
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
#[test]
fn test_startup_onboarding_preserves_explicit_tui_intent() {
    let config = setup_config_for_startup_onboarding(RuntimeEntryMode::Tui, None);

    assert_eq!(config.ui_mode, UiMode::Tui);
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
#[test]
fn test_runtime_entry_mode_follows_setup_continuation_not_renderer() {
    assert_eq!(
        runtime_entry_mode_from_setup_continuation(&thinclaw_app::SetupContinuation::Tui),
        RuntimeEntryMode::Tui
    );
    assert_eq!(
        runtime_entry_mode_from_setup_continuation(&thinclaw_app::SetupContinuation::Run),
        RuntimeEntryMode::Cli
    );
    assert_eq!(
        runtime_entry_mode_from_setup_continuation(&thinclaw_app::SetupContinuation::Exit),
        RuntimeEntryMode::Default
    );

    let ask = setup_config_for_startup_onboarding(
        RuntimeEntryMode::Cli,
        Some("preserve this request".to_string()),
    );
    assert!(matches!(
        ask.invocation.continuation,
        thinclaw_app::SetupContinuation::Ask(thinclaw_app::SetupAskRequest { ref text })
            if text == "preserve this request"
    ));
    assert_eq!(
        runtime_entry_mode_from_setup_continuation(&ask.invocation.continuation),
        RuntimeEntryMode::Cli
    );
}
