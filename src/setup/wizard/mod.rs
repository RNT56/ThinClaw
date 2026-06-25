//! Main setup wizard orchestration.
//!
//! This module is a façade: it owns the core [`SetupWizard`] state struct, its
//! constructors, and the checkpoint/persistence lifecycle helpers, then
//! delegates each cohesive concern to a focused submodule:
//!
//! - [`flow`] — UI-mode resolution, plan execution, step dispatch, back-nav
//! - [`profile`] — onboarding profile selection and profile-driven defaults
//! - [`verification`] — channel continuity and non-destructive verification
//! - [`readiness`] — readiness summary, validation items, follow-up bookkeeping
//! - [`reconnect`] — database reconnection for single-step modes
//! - plus the existing step modules (`agent`, `automation`, `channels_step`,
//!   `extensions`, `infrastructure`, `llm`, `persistence`, `presentation`,
//!   `sandbox`, `summary`, `tui_shell`) and shared `helpers`/`contracts`.

use std::{collections::BTreeMap, sync::Arc};

use secrecy::SecretString;

use crate::secrets::SecretsCrypto;
use crate::settings::{OnboardingFollowup, Settings};

#[allow(unused_imports)]
pub use self::contracts::{
    FollowupDraft, GuideTopic, OnboardingProfile, ReadinessSummary, StepDescriptor, StepStatus,
    UiMode, ValidationItem, ValidationLevel, WizardPhase, WizardPhaseId, WizardPlan, WizardStepId,
};

/// Setup wizard error.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Channel setup error: {0}")]
    Channel(String),

    #[error("User cancelled")]
    Cancelled,
}

impl From<crate::setup::channels::ChannelSetupError> for SetupError {
    fn from(e: crate::setup::channels::ChannelSetupError) -> Self {
        SetupError::Channel(e.to_string())
    }
}

/// Setup wizard configuration.
#[derive(Debug, Clone)]
pub struct SetupConfig {
    /// Skip authentication step (use existing session).
    pub skip_auth: bool,
    /// Only reconfigure channels.
    pub channels_only: bool,
    /// Preferred onboarding UI mode.
    pub ui_mode: UiMode,
    /// Optional guided settings topic.
    pub guide_topic: Option<GuideTopic>,
    /// Optional profile supplied by the CLI.
    pub profile: Option<OnboardingProfile>,
    /// When true, save settings and return without continuing into runtime.
    pub pause_after_completion: bool,
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            skip_auth: false,
            channels_only: false,
            ui_mode: UiMode::Auto,
            guide_topic: None,
            profile: None,
            pause_after_completion: false,
        }
    }
}

/// Interactive setup wizard for ThinClaw.
///
/// Fields are private but remain accessible to the wizard's child submodules
/// (Rust private visibility extends to descendant modules), so the per-concern
/// `impl SetupWizard` blocks in `flow`, `profile`, etc. can use them without
/// widening the API.
pub struct SetupWizard {
    config: SetupConfig,
    settings: Settings,

    /// Database pool (created during setup, postgres only).
    #[cfg(feature = "postgres")]
    db_pool: Option<deadpool_postgres::Pool>,
    /// libSQL backend (created during setup, libsql only).
    #[cfg(feature = "libsql")]
    db_backend: Option<crate::db::libsql::LibSqlBackend>,
    /// Secrets crypto (created during setup).
    secrets_crypto: Option<Arc<SecretsCrypto>>,
    /// Cached API key from provider setup (used by model fetcher without env mutation).
    llm_api_key: Option<SecretString>,
    /// Selected onboarding profile for the current run.
    selected_profile: OnboardingProfile,
    /// Shared step/phase plan for the current onboarding run.
    plan: Option<WizardPlan>,
    /// Live execution status per step for progress UIs.
    step_statuses: BTreeMap<WizardStepId, StepStatus>,
    /// Ephemeral follow-up tasks collected during the current run.
    followups: Vec<FollowupDraft>,
    /// Latest non-destructive channel verification readiness map.
    verified_channels: BTreeMap<String, bool>,
    /// Quick-setup primary channel selection for notification defaults.
    quick_primary_channel: Option<String>,
    /// Generated env-backed secrets master key when no secure store is available.
    generated_env_master_key: Option<String>,
    /// Actual prompt/runtime mode chosen for this onboarding run.
    resolved_ui_mode: UiMode,
}

#[derive(Clone)]
struct WizardCheckpoint {
    settings: Settings,
    #[cfg(feature = "postgres")]
    db_pool: Option<deadpool_postgres::Pool>,
    #[cfg(feature = "libsql")]
    db_backend: Option<crate::db::libsql::LibSqlBackend>,
    secrets_crypto: Option<Arc<SecretsCrypto>>,
    llm_api_key: Option<SecretString>,
    selected_profile: OnboardingProfile,
    step_statuses: BTreeMap<WizardStepId, StepStatus>,
    followups: Vec<FollowupDraft>,
    verified_channels: BTreeMap<String, bool>,
    quick_primary_channel: Option<String>,
    generated_env_master_key: Option<String>,
}

impl SetupWizard {
    /// Create a new setup wizard.
    pub fn new() -> Self {
        Self {
            config: SetupConfig::default(),
            settings: Settings::default(),
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
            selected_profile: OnboardingProfile::default(),
            plan: None,
            step_statuses: BTreeMap::new(),
            followups: Vec::new(),
            verified_channels: BTreeMap::new(),
            quick_primary_channel: None,
            generated_env_master_key: None,
            resolved_ui_mode: UiMode::Cli,
        }
    }

    /// Create a wizard with custom configuration.
    pub fn with_config(config: SetupConfig) -> Self {
        let selected_profile = config.profile.unwrap_or_default();
        Self {
            config,
            settings: Settings::default(),
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
            selected_profile,
            plan: None,
            step_statuses: BTreeMap::new(),
            followups: Vec::new(),
            verified_channels: BTreeMap::new(),
            quick_primary_channel: None,
            generated_env_master_key: None,
            resolved_ui_mode: UiMode::Cli,
        }
    }

    pub(super) fn persist_followups(&mut self) {
        self.settings.onboarding_followups = self
            .followups
            .iter()
            .map(|item| OnboardingFollowup {
                id: item.id.clone(),
                title: item.title.clone(),
                category: item.category,
                status: item.status,
                instructions: item.instructions.clone(),
                action_hint: item.action_hint.clone(),
            })
            .collect();
    }

    fn checkpoint(&self) -> WizardCheckpoint {
        WizardCheckpoint {
            settings: self.settings.clone(),
            #[cfg(feature = "postgres")]
            db_pool: self.db_pool.clone(),
            #[cfg(feature = "libsql")]
            db_backend: self.db_backend.clone(),
            secrets_crypto: self.secrets_crypto.clone(),
            llm_api_key: self.llm_api_key.clone(),
            selected_profile: self.selected_profile,
            step_statuses: self.step_statuses.clone(),
            followups: self.followups.clone(),
            verified_channels: self.verified_channels.clone(),
            quick_primary_channel: self.quick_primary_channel.clone(),
            generated_env_master_key: self.generated_env_master_key.clone(),
        }
    }

    fn restore_checkpoint(&mut self, checkpoint: WizardCheckpoint) {
        self.settings = checkpoint.settings;
        #[cfg(feature = "postgres")]
        {
            self.db_pool = checkpoint.db_pool;
        }
        #[cfg(feature = "libsql")]
        {
            self.db_backend = checkpoint.db_backend;
        }
        self.secrets_crypto = checkpoint.secrets_crypto;
        self.llm_api_key = checkpoint.llm_api_key;
        self.selected_profile = checkpoint.selected_profile;
        self.step_statuses = checkpoint.step_statuses;
        self.followups = checkpoint.followups;
        self.verified_channels = checkpoint.verified_channels;
        self.quick_primary_channel = checkpoint.quick_primary_channel;
        self.generated_env_master_key = checkpoint.generated_env_master_key;
    }
}

// Core orchestration is split into sub-modules by concern.
mod flow;
mod profile;
mod readiness;
mod reconnect;
mod verification;

// Step implementations are split into sub-modules by concern.
mod agent;
mod automation;
mod channels_step;
mod contracts;
mod extensions;
pub(crate) mod helpers;
mod infrastructure;
mod llm;
mod persistence;
mod presentation;
mod sandbox;
mod summary;
mod tui_shell;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::tempdir;

    use crate::channels::wasm::{ChannelCapabilitiesFile, available_channel_names};
    use crate::config::helpers::lock_env;

    use super::helpers::*;
    use super::*;

    #[test]
    fn test_wizard_creation() {
        let wizard = SetupWizard::new();
        assert!(!wizard.config.skip_auth);
        assert!(!wizard.config.channels_only);
        assert_eq!(wizard.config.ui_mode, UiMode::Auto);
    }

    #[test]
    fn test_wizard_with_config() {
        let config = SetupConfig {
            skip_auth: true,
            channels_only: false,
            ui_mode: UiMode::Cli,
            guide_topic: None,
            profile: None,
            pause_after_completion: false,
        };
        let wizard = SetupWizard::with_config(config);
        assert!(wizard.config.skip_auth);
    }

    #[test]
    fn test_default_onboarding_continues_into_runtime() {
        let wizard = SetupWizard::new();
        assert!(wizard.should_continue_to_runtime());
        assert_eq!(wizard.primary_runtime_command(), "thinclaw");
    }

    #[test]
    fn test_quick_setup_plan_uses_documented_twelve_steps() {
        let wizard = SetupWizard::new();
        let plan = wizard.build_plan();

        assert_eq!(plan.steps.len(), 12);
        assert!(
            !plan
                .steps
                .iter()
                .any(|step| step.id == WizardStepId::SmartRouting)
        );
        assert!(
            plan.steps
                .iter()
                .any(|step| step.id == WizardStepId::CodingWorkers)
        );
    }

    #[test]
    fn test_tui_back_reopens_menus_on_first_guided_step() {
        let mut wizard = SetupWizard::new();
        wizard.config.guide_topic = Some(GuideTopic::Ai);

        assert!(wizard.should_reopen_tui_menus_on_back(0, true));
    }

    #[test]
    fn test_tui_back_does_not_reopen_menus_after_first_step() {
        let mut wizard = SetupWizard::new();
        wizard.config.guide_topic = Some(GuideTopic::Ai);

        assert!(!wizard.should_reopen_tui_menus_on_back(1, true));
    }

    #[test]
    fn test_tui_back_reopens_menus_in_quick_setup() {
        let wizard = SetupWizard::new();

        // Quick setup should also allow Ctrl+B back to the entry prompt.
        assert!(wizard.should_reopen_tui_menus_on_back(0, true));
    }

    #[test]
    fn test_quick_notification_defaults_use_verified_telegram_owner() {
        let mut wizard = SetupWizard::new();
        wizard.quick_primary_channel = Some("telegram".to_string());
        wizard
            .verified_channels
            .insert("telegram".to_string(), true);
        wizard.settings.channels.telegram_owner_id = Some(684480568);

        wizard.apply_quick_notification_defaults();

        assert_eq!(
            wizard.settings.notifications.preferred_channel.as_deref(),
            Some("telegram")
        );
        assert_eq!(
            wizard.settings.notifications.recipient.as_deref(),
            Some("684480568")
        );
        assert!(wizard.settings.heartbeat.enabled);
        assert_eq!(
            wizard.settings.heartbeat.notify_channel.as_deref(),
            Some("telegram")
        );
    }

    #[tokio::test]
    async fn test_quick_web_channel_verification_is_ready() {
        let mut wizard = SetupWizard::new();
        wizard.quick_primary_channel = Some("web".to_string());

        let issues = wizard.step_channel_verification().await.unwrap();

        assert_eq!(issues, 0);
        assert_eq!(wizard.verified_channels.get("web"), Some(&true));
    }

    #[test]
    fn test_custom_advanced_profile_metadata() {
        assert_eq!(
            OnboardingProfile::CustomAdvanced.title(),
            "Custom / Advanced"
        );
        assert!(
            OnboardingProfile::CustomAdvanced
                .description()
                .contains("neutral baseline")
        );
    }

    #[test]
    fn test_custom_advanced_profile_preserves_existing_settings() {
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::CustomAdvanced;
        wizard.settings.skills_enabled = false;
        wizard.settings.observability_backend = "log".to_string();
        wizard.settings.providers.smart_routing_enabled = true;
        wizard.settings.providers.routing_mode = crate::settings::RoutingMode::Policy;
        wizard.settings.routines_enabled = false;
        wizard.settings.heartbeat.enabled = true;
        wizard.settings.llm_backend = Some("openai".to_string());

        wizard.apply_profile_defaults();

        assert!(!wizard.settings.skills_enabled);
        assert_eq!(wizard.settings.observability_backend, "log");
        assert!(wizard.settings.providers.smart_routing_enabled);
        assert_eq!(
            wizard.settings.providers.routing_mode,
            crate::settings::RoutingMode::Policy
        );
        assert!(!wizard.settings.routines_enabled);
        assert!(wizard.settings.heartbeat.enabled);
        assert_eq!(wizard.settings.llm_backend.as_deref(), Some("openai"));
    }

    #[test]
    fn test_remote_provider_followup_added_for_remote_primary() {
        let mut wizard = SetupWizard::new();
        wizard.settings.providers.primary = Some("openai".to_string());

        wizard.ensure_remote_provider_followup();

        assert!(wizard.followups.iter().any(|f| f.id == "provider-auth"));
    }

    #[test]
    fn test_remote_provider_followup_skipped_for_local_only() {
        let mut wizard = SetupWizard::new();
        wizard.settings.providers.primary = Some("openai".to_string());
        wizard.ensure_remote_provider_followup();
        assert!(wizard.followups.iter().any(|f| f.id == "provider-auth"));

        wizard.settings.providers.primary = Some("ollama".to_string());
        wizard.settings.providers.enabled = vec!["ollama".to_string()];
        wizard.ensure_remote_provider_followup();

        assert!(!wizard.followups.iter().any(|f| f.id == "provider-auth"));
    }

    #[test]
    fn test_remote_provider_followup_kept_for_remote_fallback() {
        let mut wizard = SetupWizard::new();
        wizard.settings.providers.primary = Some("ollama".to_string());
        wizard.settings.providers.enabled = vec!["ollama".to_string(), "openai".to_string()];

        wizard.ensure_remote_provider_followup();

        assert!(wizard.followups.iter().any(|f| f.id == "provider-auth"));
    }

    #[test]
    #[cfg(feature = "postgres")]
    fn test_mask_password_in_url() {
        assert_eq!(
            mask_password_in_url("postgres://user:secret@localhost/db"),
            "postgres://user:****@localhost/db"
        );

        // URL without password
        assert_eq!(
            mask_password_in_url("postgres://localhost/db"),
            "postgres://localhost/db"
        );
    }

    #[test]
    fn test_capitalize_first() {
        assert_eq!(capitalize_first("telegram"), "Telegram");
        assert_eq!(capitalize_first("CAPS"), "CAPS");
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn test_mask_api_key() {
        assert_eq!(
            mask_api_key("sk-ant-api03-abcdef1234567890"),
            "sk-ant...7890"
        );
        assert_eq!(mask_api_key("short"), "shor...");
        assert_eq!(mask_api_key("exactly12ch"), "exac...");
        assert_eq!(mask_api_key("exactly12chr"), "exactl...2chr");
        assert_eq!(mask_api_key(""), "...");
        // Multi-byte chars should not panic
        assert_eq!(mask_api_key("日本語キー"), "日本語キ...");
    }

    #[tokio::test]
    async fn test_install_missing_bundled_channels_installs_telegram() {
        // WASM artifacts only exist in dev builds (not CI). Skip gracefully
        // rather than fail when the telegram channel hasn't been compiled.
        if !available_channel_names().contains(&"telegram") {
            eprintln!("skipping: telegram WASM artifacts not built");
            return;
        }

        let dir = tempdir().unwrap();
        let installed = HashSet::<String>::new();

        install_missing_bundled_channels(dir.path(), &installed)
            .await
            .unwrap();

        assert!(dir.path().join("telegram.wasm").exists());
        assert!(dir.path().join("telegram.capabilities.json").exists());
    }

    #[test]
    fn test_build_channel_options_includes_available_when_missing() {
        let discovered = Vec::new();
        let options = build_channel_options(&discovered);
        let available = available_channel_names();
        // All available (built) channels should appear
        for name in &available {
            assert!(
                options.contains(&name.to_string()),
                "expected '{}' in options",
                name
            );
        }
    }

    #[test]
    fn test_build_channel_options_dedupes_available() {
        let discovered = vec![(String::from("telegram"), ChannelCapabilitiesFile::default())];
        let options = build_channel_options(&discovered);
        // telegram should appear exactly once despite being both discovered and available
        assert_eq!(
            options.iter().filter(|n| *n == "telegram").count(),
            1,
            "telegram should not be duplicated"
        );
    }

    #[test]
    fn test_claude_code_key_enable_anthropic_provider_without_changing_primary() {
        let mut wizard = SetupWizard::new();
        wizard.settings.providers.primary = Some("openai".to_string());

        wizard.enable_anthropic_provider_for_claude_code_key();

        assert_eq!(wizard.settings.providers.primary.as_deref(), Some("openai"));
        assert!(
            wizard
                .settings
                .providers
                .enabled
                .iter()
                .any(|slug| slug == "anthropic")
        );

        let slots = wizard
            .settings
            .providers
            .provider_models
            .get("anthropic")
            .expect("anthropic slots should be created");
        assert_eq!(slots.primary.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(slots.cheap.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn test_builder_and_coding_profile_enforces_advisor_executor_defaults() {
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::BuilderAndCoding;
        wizard.settings.providers.advisor_max_calls = 1;

        wizard.apply_profile_defaults();

        assert!(wizard.settings.providers.smart_routing_enabled);
        assert_eq!(
            wizard.settings.providers.routing_mode,
            crate::settings::RoutingMode::AdvisorExecutor
        );
        assert_eq!(wizard.settings.providers.advisor_max_calls, 4);
    }

    #[test]
    fn test_remote_profile_applies_service_safe_gateway_defaults() {
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::RemoteServer;

        wizard.apply_profile_defaults();

        assert_eq!(wizard.settings.channels.cli_enabled, Some(false));
        assert_eq!(wizard.settings.channels.gateway_enabled, Some(true));
        assert_eq!(
            wizard.settings.channels.gateway_host.as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(wizard.settings.channels.gateway_port, Some(3000));
        assert!(
            wizard
                .settings
                .channels
                .gateway_auth_token
                .as_deref()
                .is_some_and(|token| token.len() >= 32)
        );
        assert_eq!(wizard.settings.database_backend.as_deref(), Some("libsql"));
    }

    #[test]
    fn test_pi_os_lite_profile_applies_headless_remote_defaults() {
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::PiOsLite64;
        wizard.settings.desktop_autonomy.enabled = true;
        wizard.settings.desktop_autonomy.profile =
            crate::settings::DesktopAutonomyProfile::RecklessDesktop;

        wizard.apply_profile_defaults();

        assert_eq!(wizard.settings.channels.cli_enabled, Some(false));
        assert_eq!(wizard.settings.channels.gateway_enabled, Some(true));
        assert_eq!(
            wizard.settings.channels.gateway_host.as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(wizard.settings.channels.gateway_port, Some(3000));
        assert_eq!(wizard.settings.database_backend.as_deref(), Some("libsql"));
        assert!(!wizard.settings.desktop_autonomy.enabled);
        assert_eq!(
            wizard.settings.desktop_autonomy.profile,
            crate::settings::DesktopAutonomyProfile::Off
        );
    }

    #[test]
    fn test_cli_supplied_remote_profile_is_preselected() {
        let wizard = SetupWizard::with_config(SetupConfig {
            profile: Some(OnboardingProfile::RemoteServer),
            ..SetupConfig::default()
        });

        assert_eq!(wizard.selected_profile, OnboardingProfile::RemoteServer);
    }

    #[test]
    fn test_remote_bootstrap_env_writes_gateway_and_cli_keys() {
        let temp = tempdir().expect("temp thinclaw home");
        let _guard = EnvGuard::set("THINCLAW_HOME", temp.path().to_string_lossy().into_owned());
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::RemoteServer;
        wizard.apply_profile_defaults();
        wizard.settings.onboard_completed = true;

        wizard.write_bootstrap_env().expect("write bootstrap env");

        let env_path = temp.path().join(".env");
        let content = std::fs::read_to_string(env_path).expect("read bootstrap env");
        assert!(content.contains("GATEWAY_ENABLED=\"true\""));
        assert!(content.contains("GATEWAY_HOST=\"127.0.0.1\""));
        assert!(content.contains("GATEWAY_PORT=\"3000\""));
        assert!(content.contains("GATEWAY_AUTH_TOKEN=\""));
        assert!(content.contains("CLI_ENABLED=\"false\""));
    }

    #[test]
    fn test_pi_os_lite_bootstrap_env_writes_headless_runtime_markers() {
        let temp = tempdir().expect("temp thinclaw home");
        let _guard = EnvGuard::set("THINCLAW_HOME", temp.path().to_string_lossy().into_owned());
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::PiOsLite64;
        wizard.apply_profile_defaults();
        wizard.settings.onboard_completed = true;

        wizard.write_bootstrap_env().expect("write bootstrap env");

        let env_path = temp.path().join(".env");
        let content = std::fs::read_to_string(env_path).expect("read bootstrap env");
        assert!(content.contains("THINCLAW_RUNTIME_PROFILE=\"pi-os-lite-64\""));
        assert!(content.contains("THINCLAW_HEADLESS=\"true\""));
        assert!(content.contains("GATEWAY_ENABLED=\"true\""));
        assert!(content.contains("CLI_ENABLED=\"false\""));
    }

    #[tokio::test]
    async fn test_fetch_anthropic_models_static_fallback() {
        // With no API key, should return static defaults
        let _guard = EnvGuard::clear("ANTHROPIC_API_KEY");
        let models = fetch_anthropic_models(None).await;
        assert!(!models.is_empty());
        assert!(
            models.iter().any(|(id, _)| id.contains("claude")),
            "static defaults should include a Claude model"
        );
    }

    #[tokio::test]
    async fn test_fetch_openai_models_static_fallback() {
        let _guard = EnvGuard::clear("OPENAI_API_KEY");
        let models = fetch_openai_models(None).await;
        assert!(!models.is_empty());
        assert_eq!(models[0].0, "gpt-5.3-codex");
        assert!(
            models.iter().any(|(id, _)| id.contains("gpt")),
            "static defaults should include a GPT model"
        );
    }

    #[test]
    fn test_is_openai_chat_model_includes_gpt5_and_filters_non_chat_variants() {
        assert!(is_openai_chat_model("gpt-5"));
        assert!(is_openai_chat_model("gpt-5-mini-2026-01-01"));
        assert!(is_openai_chat_model("o3-2025-04-16"));
        assert!(!is_openai_chat_model("chatgpt-image-latest"));
        assert!(!is_openai_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_openai_chat_model("gpt-4o-mini-transcribe"));
        assert!(!is_openai_chat_model("text-embedding-3-large"));
    }

    #[test]
    fn test_sort_openai_models_prioritizes_best_models_first() {
        let mut models = vec![
            ("gpt-4o-mini".to_string(), "gpt-4o-mini".to_string()),
            ("gpt-5-mini".to_string(), "gpt-5-mini".to_string()),
            ("o3".to_string(), "o3".to_string()),
            ("gpt-4.1".to_string(), "gpt-4.1".to_string()),
            ("gpt-5".to_string(), "gpt-5".to_string()),
        ];

        sort_openai_models(&mut models);

        let ordered: Vec<String> = models.into_iter().map(|(id, _)| id).collect();
        assert_eq!(
            ordered,
            vec![
                "gpt-5".to_string(),
                "gpt-5-mini".to_string(),
                "o3".to_string(),
                "gpt-4.1".to_string(),
                "gpt-4o-mini".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn test_fetch_ollama_models_unreachable_fallback() {
        // Point at a port nothing listens on
        let models = fetch_ollama_models("http://127.0.0.1:1").await;
        assert!(!models.is_empty(), "should fall back to static defaults");
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_empty_dir() {
        let dir = tempdir().unwrap();
        let channels = discover_wasm_channels(dir.path()).await;
        assert!(channels.is_empty());
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_nonexistent_dir() {
        let channels =
            discover_wasm_channels(std::path::Path::new("/tmp/thinclaw_nonexistent_dir")).await;
        assert!(channels.is_empty());
    }

    /// RAII guard that sets/clears an env var for the duration of a test.
    struct EnvGuard {
        _env_guard: std::sync::MutexGuard<'static, ()>,
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: String) -> Self {
            let env_guard = lock_env();
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                _env_guard: env_guard,
                key,
                original,
            }
        }

        fn clear(key: &'static str) -> Self {
            let env_guard = lock_env();
            let original = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            Self {
                _env_guard: env_guard,
                key,
                original,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref val) = self.original {
                    std::env::set_var(self.key, val);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }
}
