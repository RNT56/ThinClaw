//! Main setup wizard orchestration.

use std::{collections::BTreeMap, io::IsTerminal, sync::Arc};

use secrecy::{ExposeSecret, SecretString};

use crate::secrets::SecretsCrypto;
use crate::settings::{
    OnboardingFollowup, OnboardingFollowupCategory, OnboardingFollowupStatus, Settings,
};
use crate::setup::prompts::{
    PromptUiMode as PromptRenderMode, begin_tui_prompt_session, current_prompt_ui_mode,
    print_header, print_info, print_phase_banner, print_step, print_success, print_warning,
    push_prompt_ui_mode, select_one,
};

pub use self::contracts::{
    FollowupDraft, OnboardingProfile, ReadinessSummary, StepDescriptor, StepStatus, UiMode,
    ValidationItem, ValidationLevel, WizardPhase, WizardPhaseId, WizardPlan, WizardStepId,
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
#[derive(Debug, Clone, Default)]
pub struct SetupConfig {
    /// Skip authentication step (use existing session).
    pub skip_auth: bool,
    /// Only reconfigure channels.
    pub channels_only: bool,
    /// Preferred onboarding UI mode.
    pub ui_mode: UiMode,
}

/// Interactive setup wizard for ThinClaw.
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
        }
    }

    /// Create a wizard with custom configuration.
    pub fn with_config(config: SetupConfig) -> Self {
        Self {
            config,
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
        }
    }

    fn resolve_ui_mode(&self) -> UiMode {
        match self.config.ui_mode {
            UiMode::Auto => {
                let stdin_is_tty = std::io::stdin().is_terminal();
                let stdout_is_tty = std::io::stdout().is_terminal();
                let fits_shell = crossterm::terminal::size()
                    .map(|(width, height)| width >= 100 && height >= 28)
                    .unwrap_or(false);
                if stdin_is_tty && stdout_is_tty && fits_shell {
                    UiMode::Tui
                } else {
                    UiMode::Cli
                }
            }
            mode => mode,
        }
    }

    fn build_plan(&self) -> WizardPlan {
        use WizardPhaseId as Phase;
        use WizardStepId as Step;

        let mut phases = Vec::new();
        let mut steps = Vec::new();

        let mut push_step = |id, phase_id, title, description, why_this_matters, recommended| {
            steps.push(StepDescriptor {
                id,
                phase_id,
                title,
                description,
                why_this_matters,
                recommended,
            });
        };

        if self.config.channels_only {
            phases.push(WizardPhase {
                id: Phase::ChannelsContinuity,
                title: Phase::ChannelsContinuity.title(),
                description: Phase::ChannelsContinuity.description(),
                step_ids: vec![Step::Channels, Step::ChannelVerification],
            });
            phases.push(WizardPhase {
                id: Phase::Finish,
                title: Phase::Finish.title(),
                description: Phase::Finish.description(),
                step_ids: vec![Step::Summary],
            });

            push_step(
                Step::Channels,
                Phase::ChannelsContinuity,
                "Channel Configuration",
                "Choose where ThinClaw should receive and send messages.",
                "A working channel is what turns configuration into a usable assistant.",
                Some("Start with one channel you can verify right now."),
            );
            push_step(
                Step::ChannelVerification,
                Phase::ChannelsContinuity,
                "Channel Verification",
                "Run safe checks against every enabled channel and capture any gaps.",
                "It is better to leave with one confirmed route than several unverified ones.",
                Some("Treat verification as the release gate for channel setup."),
            );
            push_step(
                Step::Summary,
                Phase::Finish,
                "Finish",
                "Review readiness, deferred tasks, and what happens next.",
                "The goal is a confident handoff into normal startup, not more guesswork.",
                None,
            );

            return WizardPlan { phases, steps };
        }

        phases.push(WizardPhase {
            id: Phase::WelcomeProfile,
            title: Phase::WelcomeProfile.title(),
            description: Phase::WelcomeProfile.description(),
            step_ids: vec![Step::Welcome, Step::Profile],
        });
        phases.push(WizardPhase {
            id: Phase::CoreRuntime,
            title: Phase::CoreRuntime.title(),
            description: Phase::CoreRuntime.description(),
            step_ids: vec![Step::Database, Step::Security],
        });
        phases.push(WizardPhase {
            id: Phase::AiStack,
            title: Phase::AiStack.title(),
            description: Phase::AiStack.description(),
            step_ids: vec![
                Step::InferenceProvider,
                Step::ModelSelection,
                Step::SmartRouting,
                Step::FallbackProviders,
                Step::Embeddings,
            ],
        });
        phases.push(WizardPhase {
            id: Phase::IdentityPresence,
            title: Phase::IdentityPresence.title(),
            description: Phase::IdentityPresence.description(),
            step_ids: vec![Step::AgentIdentity, Step::Timezone],
        });
        phases.push(WizardPhase {
            id: Phase::ChannelsContinuity,
            title: Phase::ChannelsContinuity.title(),
            description: Phase::ChannelsContinuity.description(),
            step_ids: vec![
                Step::Channels,
                Step::ChannelContinuity,
                Step::ChannelVerification,
                Step::Notifications,
            ],
        });
        phases.push(WizardPhase {
            id: Phase::CapabilitiesAutomation,
            title: Phase::CapabilitiesAutomation.title(),
            description: Phase::CapabilitiesAutomation.description(),
            step_ids: vec![
                Step::Extensions,
                Step::DockerSandbox,
                Step::ClaudeCode,
                Step::CodexCode,
                Step::ToolApproval,
                Step::Routines,
                Step::Skills,
                Step::Heartbeat,
            ],
        });
        phases.push(WizardPhase {
            id: Phase::ExperienceOperations,
            title: Phase::ExperienceOperations.title(),
            description: Phase::ExperienceOperations.description(),
            step_ids: vec![Step::WebUi, Step::Observability],
        });
        phases.push(WizardPhase {
            id: Phase::Finish,
            title: Phase::Finish.title(),
            description: Phase::Finish.description(),
            step_ids: vec![Step::Summary],
        });

        push_step(
            Step::Welcome,
            Phase::WelcomeProfile,
            "Let's Set Up ThinClaw",
            "Get a quick overview of what this flow will configure and how resume works.",
            "A clear start reduces mistakes and makes every next decision easier.",
            Some("If you are unsure, keep the recommended options and adjust later."),
        );
        push_step(
            Step::Profile,
            Phase::WelcomeProfile,
            "Choose Your Setup Style",
            "Pick a profile to prefill practical defaults for your environment.",
            "Profiles speed up setup without taking away your ability to review each section.",
            Some("Balanced is the best default for most operators."),
        );
        push_step(
            Step::Database,
            Phase::CoreRuntime,
            "Storage Foundation",
            "Choose where ThinClaw stores settings, history, and runtime state.",
            "This storage path underpins everything else in onboarding.",
            Some("libSQL + local file is the fastest reliable path for day one."),
        );
        push_step(
            Step::Security,
            Phase::CoreRuntime,
            "Secret Protection",
            "Choose how API keys and sensitive values are protected.",
            "Trust boundaries should be explicit before provider credentials are stored.",
            Some("Use your OS secure store when available."),
        );
        push_step(
            Step::InferenceProvider,
            Phase::AiStack,
            "Primary Model Provider",
            "Pick the provider ThinClaw should rely on for most requests.",
            "This choice impacts quality, latency, auth, and operating cost.",
            Some("Start with one reliable provider, then add fallback later."),
        );
        push_step(
            Step::ModelSelection,
            Phase::AiStack,
            "Primary Model",
            "Choose the main model for complex reasoning and tool planning.",
            "This model defines the quality ceiling for everyday operation.",
            None,
        );
        push_step(
            Step::SmartRouting,
            Phase::AiStack,
            "Routing Strategy",
            "Choose how ThinClaw distributes work across configured models.",
            "Routing directly shapes speed, cost, and answer quality.",
            Some("Cheap split is the recommended default for most users."),
        );
        push_step(
            Step::FallbackProviders,
            Phase::AiStack,
            "Resilience Fallbacks",
            "Add secondary providers so routing can recover when the primary path is unavailable.",
            "Fallbacks improve uptime and reduce single-provider risk.",
            Some("Add one fallback after the primary route is confirmed."),
        );
        push_step(
            Step::Embeddings,
            Phase::AiStack,
            "Memory & Semantic Search",
            "Configure embeddings so ThinClaw can search memory semantically.",
            "Good embeddings improve recall quality and reduce repetitive prompting.",
            Some("Enable this once local or remote embeddings are actually reachable."),
        );
        push_step(
            Step::AgentIdentity,
            Phase::IdentityPresence,
            "Agent Name & Presence",
            "Set the agent name and how it introduces itself to users.",
            "Identity details shape trust and consistency across channels.",
            None,
        );
        push_step(
            Step::Timezone,
            Phase::IdentityPresence,
            "Timezone",
            "Confirm the local timezone for schedules and time-aware logic.",
            "Timezone errors cause confusing routine timing and alert windows.",
            None,
        );
        push_step(
            Step::Channels,
            Phase::ChannelsContinuity,
            "Channel Configuration",
            "Choose and configure the channels ThinClaw should use.",
            "Channels are the interface where users will actually meet the agent.",
            Some("Pick only channels you can verify today."),
        );
        push_step(
            Step::ChannelContinuity,
            Phase::ChannelsContinuity,
            "Cross-Channel Session Continuity",
            "Review how direct sessions synchronize across channels and devices.",
            "Understanding continuity prevents confusion when conversations move channels.",
            None,
        );
        push_step(
            Step::ChannelVerification,
            Phase::ChannelsContinuity,
            "Channel Verification",
            "Run non-destructive checks for each configured channel and capture follow-ups.",
            "Known gaps are manageable; hidden gaps break trust in production.",
            Some("Leave onboarding with at least one fully verified path."),
        );
        push_step(
            Step::Notifications,
            Phase::ChannelsContinuity,
            "Notification Preferences",
            "Choose where proactive alerts and routine results should be delivered.",
            "Useful automation depends on a destination users actually watch.",
            Some("Pick a verified channel whenever possible."),
        );
        push_step(
            Step::Extensions,
            Phase::CapabilitiesAutomation,
            "Tools & Extensions",
            "Select capability bundles and optional tools from the registry.",
            "Tooling determines what ThinClaw can do beyond chat responses.",
            Some("Use the Balanced bundle unless you need strict minimalism."),
        );
        push_step(
            Step::DockerSandbox,
            Phase::CapabilitiesAutomation,
            "Local Tools & Docker Sandbox",
            "Set trust boundaries for local commands and isolated worker execution.",
            "Early boundary choices reduce surprise and security drift later.",
            None,
        );
        push_step(
            Step::ClaudeCode,
            Phase::CapabilitiesAutomation,
            "Claude Code Sandbox",
            "Configure optional Claude Code worker integration.",
            "Only required if your workflow depends on Claude sandbox execution.",
            None,
        );
        push_step(
            Step::CodexCode,
            Phase::CapabilitiesAutomation,
            "Codex Sandbox",
            "Configure optional Codex CLI worker integration.",
            "Only required if your workflow depends on Codex sandbox execution.",
            None,
        );
        push_step(
            Step::ToolApproval,
            Phase::CapabilitiesAutomation,
            "Tool Approval Mode",
            "Choose how much autonomy ThinClaw has when executing tools.",
            "Approval mode is the core operational safety control.",
            Some("Standard approval is recommended until trust boundaries are proven."),
        );
        push_step(
            Step::Routines,
            Phase::CapabilitiesAutomation,
            "Routines",
            "Enable or defer scheduled automation tasks.",
            "Routines are optional at launch but powerful once channels are stable.",
            None,
        );
        push_step(
            Step::Skills,
            Phase::CapabilitiesAutomation,
            "Skills",
            "Enable reusable capability packs for specialized behavior.",
            "Skills increase adaptability without modifying core runtime code.",
            Some("Keep skills enabled unless you need a minimal locked-down install."),
        );
        push_step(
            Step::Heartbeat,
            Phase::CapabilitiesAutomation,
            "Background Tasks",
            "Choose whether ThinClaw runs periodic background heartbeat tasks.",
            "Heartbeat adds value after alerts and channels are fully configured.",
            Some("Keep it off on day one unless notification delivery is already verified."),
        );
        push_step(
            Step::WebUi,
            Phase::ExperienceOperations,
            "Web UI",
            "Tune the operator-facing dashboard experience.",
            "Clear UI defaults reduce operator friction and support load.",
            None,
        );
        push_step(
            Step::Observability,
            Phase::ExperienceOperations,
            "Observability",
            "Decide how much runtime telemetry and diagnostics should be emitted.",
            "Good visibility helps debugging without flooding operators with noise.",
            Some("Start lean, then raise observability once core flows are stable."),
        );
        push_step(
            Step::Summary,
            Phase::Finish,
            "Finish",
            "Review readiness, deferred tasks, and the bootstrap handoff.",
            "A strong finish gives operators confidence to launch immediately.",
            None,
        );

        WizardPlan { phases, steps }
    }

    fn reset_plan_state(&mut self) {
        self.plan = Some(self.build_plan());
        self.step_statuses.clear();
        self.followups.clear();
        self.verified_channels.clear();
        self.settings.onboarding_followups.clear();
        if let Some(plan) = &self.plan {
            for step in &plan.steps {
                self.step_statuses.insert(step.id, StepStatus::Pending);
            }
        }
    }

    fn persist_followups(&mut self) {
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

    async fn run_cli_flow(&mut self) -> Result<(), SetupError> {
        let _prompt_mode = push_prompt_ui_mode(PromptRenderMode::Cli);
        let mode_label = match current_prompt_ui_mode() {
            PromptRenderMode::Cli => "cli",
            PromptRenderMode::Tui => "tui",
        };
        print_header("ThinClaw Humanist Cockpit");
        print_info(
            "You will move through focused phases with calm recommendations and inline readiness checks.",
        );
        print_info(
            "Progress is saved as you go, so you can pause and resume without redoing the stable parts.",
        );
        print_info(&format!("Cockpit mode: {mode_label}"));
        println!();
        self.run_planned_flow(None).await
    }

    async fn run_tui_flow(&mut self) -> Result<(), SetupError> {
        let _prompt_mode = push_prompt_ui_mode(PromptRenderMode::Tui);
        let _prompt_session = begin_tui_prompt_session().map_err(SetupError::Io)?;
        let plan = self
            .plan
            .clone()
            .ok_or_else(|| SetupError::Config("Onboarding plan was not initialized".to_string()))?;
        let mut shell = tui_shell::OnboardingTuiShell::new(plan);
        shell.show_intro(self)?;
        self.run_planned_flow(Some(&mut shell)).await?;
        shell.show_completion(self)?;
        Ok(())
    }

    async fn run_planned_flow(
        &mut self,
        mut shell: Option<&mut tui_shell::OnboardingTuiShell>,
    ) -> Result<(), SetupError> {
        let plan = self
            .plan
            .clone()
            .ok_or_else(|| SetupError::Config("Onboarding plan was not initialized".to_string()))?;
        let total_steps = plan.total_steps();
        let mut last_phase = None;

        for (index, descriptor) in plan.steps.iter().enumerate() {
            if shell.is_none() && last_phase != Some(descriptor.phase_id) {
                print_phase_banner(
                    descriptor.phase_id.title(),
                    descriptor.phase_id.description(),
                );
            }
            last_phase = Some(descriptor.phase_id);

            self.step_statuses
                .insert(descriptor.id, StepStatus::InProgress);

            if let Some(shell) = shell.as_deref_mut() {
                shell.show_step(self, descriptor, index + 1, total_steps)?;
            } else {
                print_step(index + 1, total_steps, descriptor.title);
                print_info(descriptor.description);
                print_info(descriptor.why_this_matters);
                if let Some(recommended) = descriptor.recommended {
                    print_success(&format!("Recommended: {}", recommended));
                }
                println!();
            }

            let status = self.execute_step(descriptor.id).await?;
            self.step_statuses.insert(descriptor.id, status);
            self.persist_followups();

            if self.should_persist_step(descriptor.id) {
                self.persist_after_step().await;
            }

            if let Some(shell) = shell.as_deref_mut() {
                shell.show_step_result(self, descriptor, status)?;
            }
        }

        Ok(())
    }

    fn should_persist_step(&self, step_id: WizardStepId) -> bool {
        !matches!(
            step_id,
            WizardStepId::Welcome
                | WizardStepId::Profile
                | WizardStepId::ChannelContinuity
                | WizardStepId::Summary
        )
    }

    async fn execute_step(&mut self, step_id: WizardStepId) -> Result<StepStatus, SetupError> {
        match step_id {
            WizardStepId::Welcome => {
                self.step_welcome()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Profile => {
                self.step_profile()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Database => {
                self.step_database().await?;
                let step1_settings = self.settings.clone();
                self.try_load_existing_settings().await;
                self.settings.merge_from(&step1_settings);
                self.apply_profile_defaults();
                Ok(StepStatus::Completed)
            }
            WizardStepId::Security => {
                self.step_security().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::InferenceProvider => {
                if self.config.skip_auth {
                    self.step_provider_review_skip_auth().await?;
                } else {
                    self.step_inference_provider().await?;
                }
                Ok(StepStatus::Completed)
            }
            WizardStepId::ModelSelection => {
                self.step_model_selection().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::SmartRouting => {
                self.step_smart_routing().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::FallbackProviders => {
                self.step_fallback_providers().await?;
                if self.config.skip_auth {
                    self.ensure_remote_provider_followup();
                } else {
                    self.ensure_onboarding_provider_api_key().await?;
                }
                Ok(StepStatus::Completed)
            }
            WizardStepId::Embeddings => {
                self.step_embeddings()?;
                Ok(if self.settings.embeddings.enabled {
                    StepStatus::Completed
                } else {
                    StepStatus::NeedsAttention
                })
            }
            WizardStepId::AgentIdentity => {
                self.step_agent_identity()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Timezone => {
                self.step_timezone()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Channels => {
                if self.config.channels_only {
                    self.reconnect_existing_db().await?;
                }
                self.step_channels().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::ChannelContinuity => {
                self.step_channel_continuity()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::ChannelVerification => {
                let issues = self.step_channel_verification().await?;
                Ok(if issues == 0 {
                    StepStatus::Completed
                } else {
                    StepStatus::NeedsAttention
                })
            }
            WizardStepId::Notifications => {
                self.step_notification_preferences().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Extensions => {
                self.step_extensions().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::DockerSandbox => {
                self.step_docker_sandbox().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::ClaudeCode => {
                self.step_claude_code().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::CodexCode => {
                self.step_codex_code().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::ToolApproval => {
                self.step_tool_approval()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Routines => {
                self.step_routines()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Skills => {
                self.step_skills()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Heartbeat => {
                self.step_heartbeat()?;
                Ok(if self.settings.heartbeat.enabled {
                    StepStatus::Completed
                } else {
                    StepStatus::Skipped
                })
            }
            WizardStepId::WebUi => {
                self.step_web_ui()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Observability => {
                self.step_observability()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Summary => {
                self.save_and_summarize().await?;
                Ok(if self.followups.is_empty() {
                    StepStatus::Completed
                } else {
                    StepStatus::NeedsAttention
                })
            }
        }
    }

    fn readiness_summary(&self) -> ReadinessSummary {
        let ready_now = self
            .step_statuses
            .values()
            .filter(|status| matches!(status, StepStatus::Completed))
            .count();
        let needs_attention = self
            .step_statuses
            .values()
            .filter(|status| matches!(status, StepStatus::NeedsAttention))
            .count();
        let followups = self.followups.len();

        let headline = if followups == 0 && needs_attention == 0 {
            "Launch-ready".to_string()
        } else if followups > 0 || needs_attention > 0 {
            "Attention queued".to_string()
        } else {
            "Bringing systems online".to_string()
        };

        ReadinessSummary {
            ready_now,
            needs_attention,
            followups,
            headline,
        }
    }

    fn validation_items(&self) -> Vec<ValidationItem> {
        let mut items = Vec::new();

        if self.settings.database_backend.is_some() {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "Core runtime".to_string(),
                detail: self
                    .settings
                    .database_backend
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Error,
                title: "Core runtime".to_string(),
                detail: "Storage still needs to be configured before ThinClaw can fully launch."
                    .to_string(),
            });
        }

        if self.settings.llm_backend.is_some() && self.settings.selected_model.is_some() {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "AI stack".to_string(),
                detail: format!(
                    "{} / {}",
                    self.settings.llm_backend.as_deref().unwrap_or("unknown"),
                    self.settings
                        .selected_model
                        .as_deref()
                        .unwrap_or("unselected")
                ),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Error,
                title: "AI stack".to_string(),
                detail:
                    "Primary provider or model still needs review before the agent is fully ready."
                        .to_string(),
            });
        }

        let enabled_channels = self.configured_channel_names();
        if enabled_channels.is_empty() {
            items.push(ValidationItem {
                level: ValidationLevel::Warning,
                title: "Channels".to_string(),
                detail: "Only the built-in terminal path is confirmed right now.".to_string(),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "Channels".to_string(),
                detail: enabled_channels.join(", "),
            });
        }

        if !self.followups.is_empty() {
            items.push(ValidationItem {
                level: ValidationLevel::Warning,
                title: "Follow-ups".to_string(),
                detail: format!(
                    "{} follow-up item(s) are queued for after launch.",
                    self.followups.len()
                ),
            });
        }

        items
    }

    fn add_followup(&mut self, draft: FollowupDraft) {
        if let Some(existing) = self.followups.iter_mut().find(|item| item.id == draft.id) {
            *existing = draft;
        } else {
            self.followups.push(draft);
        }
    }

    fn remove_followup(&mut self, id: &str) {
        self.followups.retain(|item| item.id != id);
    }

    fn step_welcome(&self) -> Result<(), SetupError> {
        print_info(
            "Welcome to the Humanist Cockpit. We will bring ThinClaw online in focused phases instead of one long checklist.",
        );
        print_info(
            "Progress is saved incrementally, so you can pause safely and resume without losing ground.",
        );
        print_info(
            "When onboarding is complete, ThinClaw hands you back to the normal bootstrap flow automatically.",
        );
        Ok(())
    }

    fn step_profile(&mut self) -> Result<(), SetupError> {
        let options = [
            "Balanced            - recommended defaults for most first runs",
            "Local & Private     - prefer local inference and fewer outbound dependencies",
            "Builder & Coding    - optimize for tool use, coding, and advisor/executor routing",
            "Channel-First       - prioritize messaging reachability and notification setup",
            "Custom / Advanced   - neutral baseline with minimal profile-driven defaults",
        ];
        let choice =
            select_one("Which onboarding lane fits best?", &options).map_err(SetupError::Io)?;
        self.selected_profile = match choice {
            1 => OnboardingProfile::LocalAndPrivate,
            2 => OnboardingProfile::BuilderAndCoding,
            3 => OnboardingProfile::ChannelFirst,
            4 => OnboardingProfile::CustomAdvanced,
            _ => OnboardingProfile::Balanced,
        };

        if matches!(self.selected_profile, OnboardingProfile::CustomAdvanced) {
            print_success(
                "Using the Custom / Advanced profile. ThinClaw will keep profile-driven defaults light so you can make each major choice directly.",
            );
        } else {
            print_success(&format!(
                "Using the {} profile. Recommendations are prefilled, and every relevant section still stays reviewable.",
                self.selected_profile.title()
            ));
        }
        print_info(self.selected_profile.description());
        Ok(())
    }

    fn apply_profile_defaults(&mut self) {
        match self.selected_profile {
            OnboardingProfile::Balanced => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                self.settings.providers.smart_routing_enabled = true;
                if self.settings.providers.routing_mode == crate::settings::RoutingMode::PrimaryOnly
                {
                    self.settings.providers.routing_mode = crate::settings::RoutingMode::CheapSplit;
                }
                self.settings.routines_enabled = true;
                if !self.settings.heartbeat.enabled {
                    self.settings.heartbeat.enabled = false;
                }
            }
            OnboardingProfile::LocalAndPrivate => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                if self.settings.llm_backend.is_none() {
                    self.settings.llm_backend = Some("ollama".to_string());
                }
                self.settings.providers.smart_routing_enabled = false;
                self.settings.providers.routing_mode = crate::settings::RoutingMode::PrimaryOnly;
                if !self.settings.embeddings.enabled {
                    self.settings.embeddings.provider = "ollama".to_string();
                    self.settings.embeddings.model = "nomic-embed-text".to_string();
                }
                self.settings.routines_enabled = true;
                self.settings.heartbeat.enabled = false;
            }
            OnboardingProfile::BuilderAndCoding => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                self.settings.providers.smart_routing_enabled = true;
                self.settings.providers.routing_mode =
                    crate::settings::RoutingMode::AdvisorExecutor;
                self.settings.providers.advisor_max_calls =
                    self.settings.providers.advisor_max_calls.max(3);
                self.settings.routines_enabled = true;
                self.settings.heartbeat.enabled = false;
            }
            OnboardingProfile::ChannelFirst => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                self.settings.providers.smart_routing_enabled = true;
                if self.settings.providers.routing_mode == crate::settings::RoutingMode::PrimaryOnly
                {
                    self.settings.providers.routing_mode = crate::settings::RoutingMode::CheapSplit;
                }
                self.settings.routines_enabled = true;
            }
            OnboardingProfile::CustomAdvanced => {}
        }
    }

    fn configured_channel_names(&self) -> Vec<String> {
        let mut channels = Vec::new();
        if self.settings.channels.http_enabled {
            channels.push("http".to_string());
        }
        if self.settings.channels.signal_enabled {
            channels.push("signal".to_string());
        }
        if self.settings.channels.discord_enabled {
            channels.push("discord".to_string());
        }
        if self.settings.channels.slack_enabled {
            channels.push("slack".to_string());
        }
        if self.settings.channels.nostr_enabled {
            channels.push("nostr".to_string());
        }
        if self.settings.channels.gmail_enabled {
            channels.push("gmail".to_string());
        }
        #[cfg(target_os = "macos")]
        if self.settings.channels.imessage_enabled {
            channels.push("imessage".to_string());
        }
        #[cfg(target_os = "macos")]
        if self.settings.channels.apple_mail_enabled {
            channels.push("apple_mail".to_string());
        }
        channels.extend(self.settings.channels.wasm_channels.iter().cloned());
        channels
    }

    fn ensure_remote_provider_followup(&mut self) {
        let provider_needs_credentials = |provider_slug: &str| match provider_slug {
            "ollama" | "llama_cpp" => false,
            "openai_compatible" => {
                let base_url = self
                    .settings
                    .openai_compatible_base_url
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let looks_local = base_url.starts_with("http://localhost")
                    || base_url.starts_with("https://localhost")
                    || base_url.contains("127.0.0.1")
                    || base_url.contains("0.0.0.0");
                !looks_local
            }
            _ => true,
        };

        let primary_provider = self
            .settings
            .providers
            .primary
            .as_deref()
            .or(self.settings.llm_backend.as_deref());
        let primary_needs_credentials = primary_provider
            .map(provider_needs_credentials)
            .unwrap_or(false);
        let any_enabled_needs_credentials = self
            .settings
            .providers
            .enabled
            .iter()
            .any(|slug| provider_needs_credentials(slug));

        if !primary_needs_credentials && !any_enabled_needs_credentials {
            self.remove_followup("provider-auth");
            return;
        }

        self.add_followup(FollowupDraft {
            id: "provider-auth".to_string(),
            title: "Provide remote model credentials".to_string(),
            category: OnboardingFollowupCategory::Authentication,
            status: OnboardingFollowupStatus::Pending,
            instructions: "Skip-auth mode kept provider review non-secret. Add the relevant provider API key before relying on remote routing or failover.".to_string(),
            action_hint: Some("Set the provider env var or rerun `thinclaw onboard --ui cli` without --skip-auth.".to_string()),
        });
    }

    fn step_channel_continuity(&self) -> Result<(), SetupError> {
        print_info(
            "ThinClaw keeps one canonical direct-message session per linked principal across channels and devices.",
        );
        print_info(
            "That means the same person can continue a direct conversation from another channel without losing context.",
        );
        print_info(
            "Group threads stay isolated so public or shared spaces do not bleed into direct sessions.",
        );
        Ok(())
    }

    async fn step_channel_verification(&mut self) -> Result<usize, SetupError> {
        let mut issues = 0usize;
        let secrets = self.init_secrets_context().await.ok();
        self.verified_channels.clear();

        self.remove_followup("channel-verification");

        if self.configured_channel_names().is_empty() {
            self.add_followup(FollowupDraft {
                id: "channel-verification".to_string(),
                title: "No external channels verified yet".to_string(),
                category: OnboardingFollowupCategory::Verification,
                status: OnboardingFollowupStatus::Optional,
                instructions: "ThinClaw can still run locally, but no external messaging path is configured yet.".to_string(),
                action_hint: Some("Rerun `thinclaw onboard --channels-only` when you are ready to add a channel.".to_string()),
            });
            print_warning(
                "No external messaging channel is configured yet. ThinClaw will still work locally.",
            );
            return Ok(1);
        }

        if self.settings.channels.http_enabled {
            let host = self
                .settings
                .channels
                .http_host
                .as_deref()
                .unwrap_or("0.0.0.0");
            let port = self.settings.channels.http_port.unwrap_or(8080);
            let ready = !host.trim().is_empty() && port > 0;
            self.verified_channels.insert("http".to_string(), ready);
            if ready {
                print_success("HTTP channel configuration looks valid.");
            } else {
                issues += 1;
                print_warning("HTTP channel is enabled but host/port configuration is invalid.");
            }
        }

        if self.settings.channels.signal_enabled {
            let signal_ready = if let (Some(url), Some(account)) = (
                self.settings.channels.signal_http_url.as_deref(),
                self.settings.channels.signal_account.as_deref(),
            ) {
                if url.trim().is_empty() || account.trim().is_empty() {
                    false
                } else {
                    Self::verify_http_reachable(url).await
                }
            } else {
                false
            };
            self.verified_channels
                .insert("signal".to_string(), signal_ready);
            if signal_ready {
                print_success("Signal verification passed (configuration + reachability).");
            } else {
                issues += 1;
                print_warning(
                    "Signal verification failed. Check account configuration and signal-http-api reachability.",
                );
            }
        }

        if self.settings.channels.discord_enabled {
            let mut discord_token = self
                .settings
                .channels
                .discord_bot_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("DISCORD_BOT_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if discord_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("discord_bot_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    discord_token = Some(token);
                }
            }

            let discord_ready = if let Some(token) = discord_token {
                Self::verify_discord_auth(&token).await
            } else {
                false
            };
            self.verified_channels
                .insert("discord".to_string(), discord_ready);
            if discord_ready {
                print_success("Discord verification passed (bot token accepted).");
            } else {
                issues += 1;
                print_warning(
                    "Discord verification failed. Ensure the bot token is valid and reachable.",
                );
            }
        }

        if self.settings.channels.slack_enabled {
            let mut bot_token = self
                .settings
                .channels
                .slack_bot_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("SLACK_BOT_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if bot_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("slack_bot_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    bot_token = Some(token);
                }
            }

            let mut app_token = self
                .settings
                .channels
                .slack_app_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("SLACK_APP_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if app_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("slack_app_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    app_token = Some(token);
                }
            }

            let slack_ready = if let (Some(bot), Some(app)) = (bot_token, app_token) {
                Self::verify_slack_bot_auth(&bot).await && Self::verify_slack_app_auth(&app).await
            } else {
                false
            };
            self.verified_channels
                .insert("slack".to_string(), slack_ready);
            if slack_ready {
                print_success("Slack verification passed (bot + app tokens accepted).");
            } else {
                issues += 1;
                print_warning(
                    "Slack verification failed. Check bot/app tokens and workspace connectivity.",
                );
            }
        }

        if self.settings.channels.nostr_enabled {
            let relays = self
                .settings
                .channels
                .nostr_relays
                .clone()
                .unwrap_or_default();
            let nostr_ready = Self::verify_nostr_relays(&relays).await;
            self.verified_channels
                .insert("nostr".to_string(), nostr_ready);
            if nostr_ready {
                print_success("Nostr relay verification passed.");
            } else {
                issues += 1;
                print_warning(
                    "Nostr verification failed. Ensure at least one relay URL is valid and reachable.",
                );
            }
        }

        if self.settings.channels.gmail_enabled {
            let gmail_ready = self
                .settings
                .channels
                .gmail_project_id
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty())
                && self
                    .settings
                    .channels
                    .gmail_subscription_id
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty())
                && self
                    .settings
                    .channels
                    .gmail_topic_id
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty());
            self.verified_channels
                .insert("gmail".to_string(), gmail_ready);
            if gmail_ready {
                print_success("Gmail verification passed (required Pub/Sub fields present).");
            } else {
                issues += 1;
                print_warning(
                    "Gmail verification failed. Project, subscription, and topic are required.",
                );
            }
        }

        for wasm_channel in &self.settings.channels.wasm_channels {
            let ready = dirs::home_dir()
                .map(|home| {
                    home.join(".thinclaw/channels")
                        .join(format!("{wasm_channel}.wasm"))
                })
                .is_some_and(|path| path.exists());
            self.verified_channels.insert(wasm_channel.clone(), ready);
            if ready {
                print_success(&format!(
                    "WASM channel '{}' is installed and discoverable.",
                    wasm_channel
                ));
            } else {
                issues += 1;
                print_warning(&format!(
                    "WASM channel '{}' is enabled but the wasm artifact is missing.",
                    wasm_channel
                ));
            }
        }

        if issues > 0 {
            self.add_followup(FollowupDraft {
                id: "channel-verification".to_string(),
                title: "Review incomplete channel configuration".to_string(),
                category: OnboardingFollowupCategory::Verification,
                status: OnboardingFollowupStatus::NeedsAttention,
                instructions: "At least one enabled channel is still missing required configuration details or verification signals.".to_string(),
                action_hint: Some("Use `thinclaw onboard --channels-only` to revisit the channel flow.".to_string()),
            });
            print_warning(&format!(
                "Channel verification found {} configuration gap(s). The completion screen will keep a follow-up for you.",
                issues
            ));
        } else {
            print_success(
                "Channel verification found at least one ready messaging path and no obvious configuration gaps.",
            );
        }

        Ok(issues)
    }

    async fn verify_http_reachable(url: &str) -> bool {
        let parsed = match url::Url::parse(url) {
            Ok(parsed) => parsed,
            Err(_) => return false,
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return false;
        }
        reqwest::Client::new()
            .get(parsed)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }

    async fn verify_discord_auth(bot_token: &str) -> bool {
        reqwest::Client::new()
            .get("https://discord.com/api/v10/users/@me")
            .header("Authorization", format!("Bot {}", bot_token.trim()))
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    async fn verify_slack_bot_auth(bot_token: &str) -> bool {
        let response = reqwest::Client::new()
            .post("https://slack.com/api/auth.test")
            .bearer_auth(bot_token.trim())
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await;

        let Ok(response) = response else {
            return false;
        };
        let Ok(payload) = response.json::<serde_json::Value>().await else {
            return false;
        };
        payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn verify_slack_app_auth(app_token: &str) -> bool {
        let response = reqwest::Client::new()
            .post("https://slack.com/api/apps.connections.open")
            .bearer_auth(app_token.trim())
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await;

        let Ok(response) = response else {
            return false;
        };
        let Ok(payload) = response.json::<serde_json::Value>().await else {
            return false;
        };
        payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn verify_nostr_relays(relays_csv: &str) -> bool {
        for relay in relays_csv
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let Ok(url) = url::Url::parse(relay) else {
                continue;
            };
            if !matches!(url.scheme(), "ws" | "wss") {
                continue;
            }
            let Some(host) = url.host_str() else {
                continue;
            };
            let port = url
                .port_or_known_default()
                .unwrap_or(if url.scheme() == "wss" { 443 } else { 80 });
            let reachable = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                tokio::net::lookup_host((host, port)),
            )
            .await
            .ok()
            .and_then(|lookup| lookup.ok())
            .is_some_and(|mut addrs| addrs.next().is_some());
            if reachable {
                return true;
            }
        }
        false
    }

    /// Run the setup wizard.
    ///
    /// Settings are persisted incrementally after each successful step so
    /// that progress is not lost if a later step fails. On re-run, existing
    /// settings are loaded from the database after Step 1 establishes a
    /// connection, so users don't have to re-enter everything.
    pub async fn run(&mut self) -> Result<(), SetupError> {
        self.reset_plan_state();

        match self.config.ui_mode {
            UiMode::Cli => self.run_cli_flow().await,
            UiMode::Tui => self.run_tui_flow().await,
            UiMode::Auto => {
                if self.resolve_ui_mode() == UiMode::Tui {
                    match self.run_tui_flow().await {
                        Ok(()) => Ok(()),
                        Err(SetupError::Cancelled) => Err(SetupError::Cancelled),
                        Err(error) => {
                            print_warning(&format!(
                                "The onboarding TUI could not continue cleanly ({}). Falling back to the terminal wizard.",
                                error
                            ));
                            self.reset_plan_state();
                            self.run_cli_flow().await
                        }
                    }
                } else {
                    self.run_cli_flow().await
                }
            }
        }
    }

    /// Reconnect to the existing database and load settings.
    ///
    /// Used by channels-only mode (and future single-step modes) so that
    /// `init_secrets_context()` and `save_and_summarize()` have a live
    /// database connection and the wizard's `self.settings` reflects the
    /// previously saved configuration.
    async fn reconnect_existing_db(&mut self) -> Result<(), SetupError> {
        // Determine backend from env (set by bootstrap .env loaded in main).
        let backend = std::env::var("DATABASE_BACKEND").unwrap_or_else(|_| "postgres".to_string());

        // Try libsql first if that's the configured backend.
        #[cfg(feature = "libsql")]
        if backend == "libsql" || backend == "turso" || backend == "sqlite" {
            return self.reconnect_libsql().await;
        }

        // Try postgres (either explicitly configured or as default).
        #[cfg(feature = "postgres")]
        {
            let _ = &backend;
            return self.reconnect_postgres().await;
        }

        #[allow(unreachable_code)]
        Err(SetupError::Database(
            "No database configured. Run full setup first (thinclaw onboard).".to_string(),
        ))
    }

    /// Reconnect to an existing PostgreSQL database and load settings.
    #[cfg(feature = "postgres")]
    async fn reconnect_postgres(&mut self) -> Result<(), SetupError> {
        let url = std::env::var("DATABASE_URL").map_err(|_| {
            SetupError::Database(
                "DATABASE_URL not set. Run full setup first (thinclaw onboard).".to_string(),
            )
        })?;

        self.test_database_connection_postgres(&url).await?;
        self.settings.database_backend = Some("postgres".to_string());
        self.settings.database_url = Some(url.clone());

        // Load existing settings from DB, then restore connection fields that
        // may not be persisted in the settings map.
        if let Some(ref pool) = self.db_pool {
            let store = crate::history::Store::from_pool(pool.clone());
            if let Ok(map) = store.get_all_settings("default").await {
                self.settings = Settings::from_db_map(&map);
                self.settings.database_backend = Some("postgres".to_string());
                self.settings.database_url = Some(url);
            }
        }

        Ok(())
    }

    /// Reconnect to an existing libSQL database and load settings.
    #[cfg(feature = "libsql")]
    async fn reconnect_libsql(&mut self) -> Result<(), SetupError> {
        let path = std::env::var("LIBSQL_PATH").unwrap_or_else(|_| {
            crate::config::default_libsql_path()
                .to_string_lossy()
                .to_string()
        });
        let turso_url = std::env::var("LIBSQL_URL").ok();
        let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();

        self.test_database_connection_libsql(&path, turso_url.as_deref(), turso_token.as_deref())
            .await?;

        self.settings.database_backend = Some("libsql".to_string());
        self.settings.libsql_path = Some(path.clone());
        if let Some(ref url) = turso_url {
            self.settings.libsql_url = Some(url.clone());
        }

        // Load existing settings from DB, then restore connection fields that
        // may not be persisted in the settings map.
        if let Some(ref db) = self.db_backend {
            use crate::db::SettingsStore as _;
            if let Ok(map) = db.get_all_settings("default").await {
                self.settings = Settings::from_db_map(&map);
                self.settings.database_backend = Some("libsql".to_string());
                self.settings.libsql_path = Some(path);
                if let Some(url) = turso_url {
                    self.settings.libsql_url = Some(url);
                }
            }
        }

        Ok(())
    }
}

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
        };
        let wizard = SetupWizard::with_config(config);
        assert!(wizard.config.skip_auth);
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
        assert_eq!(slots.primary.as_deref(), Some("claude-sonnet-4-20250514"));
        assert!(slots.cheap.is_some());
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
