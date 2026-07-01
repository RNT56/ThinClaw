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
    assert!(!config.pause_after_completion);
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
#[test]
fn test_startup_onboarding_preserves_explicit_tui_intent() {
    let config = setup_config_for_startup_onboarding(RuntimeEntryMode::Tui);

    assert_eq!(config.ui_mode, UiMode::Tui);
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
#[test]
fn test_runtime_entry_mode_follows_resolved_wizard_ui() {
    assert_eq!(
        runtime_entry_mode_from_ui_mode(UiMode::Tui),
        RuntimeEntryMode::Tui
    );
    assert_eq!(
        runtime_entry_mode_from_ui_mode(UiMode::Cli),
        RuntimeEntryMode::Cli
    );
    assert_eq!(
        runtime_entry_mode_from_ui_mode(UiMode::Auto),
        RuntimeEntryMode::Cli
    );
}
