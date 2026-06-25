//! Wizard run orchestration: UI-mode resolution, plan execution, step
//! dispatch, and TUI back-navigation rerouting.

use crate::setup::prompts::{
    PromptUiMode as PromptRenderMode, TuiPromptContext, clear_tui_prompt_context,
    clear_tui_prompt_messages, current_prompt_ui_mode, is_back_navigation, print_header,
    print_info, print_phase_banner, print_step, print_success, print_warning, push_prompt_ui_mode,
    select_one, set_tui_prompt_context,
};
use crate::terminal_branding::set_runtime_cli_skin_override;
use std::io::IsTerminal;

use super::tui_shell;
use super::{GuideTopic, SetupError, SetupWizard, StepStatus, UiMode, WizardPlan, WizardStepId};

impl SetupWizard {
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

    fn plan_input(&self) -> thinclaw_app::SetupWizardPlanInput {
        thinclaw_app::SetupWizardPlanInput {
            channels_only: self.config.channels_only,
            guide_topic: self.config.guide_topic.map(GuideTopic::app_topic),
        }
    }

    pub(super) fn is_guide_mode(&self) -> bool {
        self.plan_input().is_guide_mode()
    }

    pub(super) fn is_quick_setup(&self) -> bool {
        self.plan_input().is_quick_setup()
    }

    pub fn runtime_ui_mode(&self) -> UiMode {
        match self.resolved_ui_mode {
            UiMode::Cli | UiMode::Tui => self.resolved_ui_mode,
            UiMode::Auto => UiMode::Cli,
        }
    }

    pub fn should_continue_to_runtime(&self) -> bool {
        !self.config.pause_after_completion
    }

    fn runtime_command_input(&self) -> thinclaw_app::SetupRuntimeCommandInput {
        thinclaw_app::SetupRuntimeCommandInput {
            profile: self.selected_profile.app_profile(),
            ui_mode: self.runtime_ui_mode().app_mode(),
            continue_to_runtime: self.should_continue_to_runtime(),
            pause_after_completion: self.config.pause_after_completion,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn primary_runtime_command(&self) -> &'static str {
        thinclaw_app::setup_primary_runtime_command(self.runtime_command_input())
    }

    pub(super) fn runtime_handoff_summary(&self) -> String {
        thinclaw_app::setup_runtime_handoff_summary(self.runtime_command_input())
    }

    pub(super) fn what_next_commands(&self) -> Vec<String> {
        thinclaw_app::setup_what_next_commands(self.runtime_command_input())
    }

    pub(super) fn build_plan(&self) -> WizardPlan {
        thinclaw_app::setup_wizard_plan(self.plan_input())
    }

    fn reset_plan_state(&mut self) {
        self.plan = Some(self.build_plan());
        self.step_statuses.clear();
        self.followups.clear();
        self.verified_channels.clear();
        self.quick_primary_channel = None;
        self.settings.onboarding_followups.clear();
        if let Some(plan) = &self.plan {
            for step in &plan.steps {
                self.step_statuses.insert(step.id, StepStatus::Pending);
            }
        }
    }

    async fn run_cli_flow(&mut self) -> Result<(), SetupError> {
        let _prompt_mode = push_prompt_ui_mode(PromptRenderMode::Cli);
        set_runtime_cli_skin_override(self.settings.agent.cli_skin.clone());
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
        crate::setup::prompts::print_blank_line();
        self.run_planned_flow(None).await
    }

    async fn run_tui_flow(&mut self) -> Result<(), SetupError> {
        let _prompt_mode = push_prompt_ui_mode(PromptRenderMode::Tui);
        set_runtime_cli_skin_override(self.settings.agent.cli_skin.clone());
        clear_tui_prompt_messages();
        clear_tui_prompt_context();
        let plan = self
            .plan
            .clone()
            .ok_or_else(|| SetupError::Config("Onboarding plan was not initialized".to_string()))?;
        let mut shell = tui_shell::OnboardingTuiShell::new(plan);
        self.run_planned_flow(Some(&mut shell)).await?;
        shell.show_completion(self)?;
        clear_tui_prompt_context();
        Ok(())
    }

    async fn run_planned_flow(
        &mut self,
        shell: Option<&mut tui_shell::OnboardingTuiShell>,
    ) -> Result<(), SetupError> {
        let mut plan = self
            .plan
            .clone()
            .ok_or_else(|| SetupError::Config("Onboarding plan was not initialized".to_string()))?;
        let mut total_steps = plan.total_steps();
        let mut last_phase = None;
        let mut index = 0usize;

        while index < total_steps {
            let descriptor = plan.steps[index].clone();
            if shell.is_some() {
                clear_tui_prompt_messages();
                set_tui_prompt_context(TuiPromptContext {
                    phase_title: Some(descriptor.phase_id.title().to_string()),
                    phase_description: Some(descriptor.phase_id.description().to_string()),
                    step_progress: Some(format!("Step {}/{}", index + 1, total_steps)),
                    description: Some(descriptor.description.to_string()),
                    why_this_matters: Some(descriptor.why_this_matters.to_string()),
                    recommended: descriptor.recommended.map(ToOwned::to_owned),
                });
            }
            if shell.is_none() && last_phase != Some(descriptor.phase_id) {
                print_phase_banner(
                    descriptor.phase_id.title(),
                    descriptor.phase_id.description(),
                );
            }
            last_phase = Some(descriptor.phase_id);

            let checkpoint = self.checkpoint();
            self.step_statuses
                .insert(descriptor.id, StepStatus::InProgress);

            if shell.is_none() {
                print_step(index + 1, total_steps, descriptor.title);
                print_info(descriptor.description);
                print_info(descriptor.why_this_matters);
                if let Some(recommended) = descriptor.recommended {
                    print_success(&format!("Recommended: {}", recommended));
                }
                crate::setup::prompts::print_blank_line();
            }

            let status = match self.execute_step(descriptor.id).await {
                Ok(status) => status,
                Err(SetupError::Io(error)) if shell.is_some() && is_back_navigation(&error) => {
                    self.restore_checkpoint(checkpoint);
                    let rerouted = if self.should_reopen_tui_menus_on_back(index, shell.is_some()) {
                        if self.is_guide_mode() {
                            self.reroute_tui_back_from_first_guided_step().await?
                        } else {
                            self.reroute_tui_back_from_first_quick_step().await?
                        }
                    } else {
                        false
                    };
                    if rerouted {
                        plan = self.plan.clone().ok_or_else(|| {
                            SetupError::Config(
                                "Onboarding plan was not initialized after back navigation"
                                    .to_string(),
                            )
                        })?;
                        total_steps = plan.total_steps();
                        index = 0;
                        last_phase = None;
                        continue;
                    }
                    index = index.saturating_sub(1);
                    last_phase = None;
                    continue;
                }
                Err(error) => return Err(error),
            };
            self.step_statuses.insert(descriptor.id, status);
            self.persist_followups();

            if self.should_persist_step(descriptor.id) {
                self.persist_after_step().await;
            }

            if shell.is_some() {
                match status {
                    StepStatus::NeedsAttention => {
                        print_warning("This step left follow-up work queued for later.");
                    }
                    StepStatus::Skipped => {
                        print_info("This step was skipped for now.");
                    }
                    _ => {}
                }
            }

            index += 1;
        }

        Ok(())
    }

    pub(super) fn should_reopen_tui_menus_on_back(&self, index: usize, has_shell: bool) -> bool {
        has_shell && index == 0 && !self.config.channels_only
    }

    async fn reroute_tui_back_from_first_guided_step(&mut self) -> Result<bool, SetupError> {
        let original_guide_topic = self.config.guide_topic;
        if !self.is_guide_mode() || !self.should_reopen_tui_menus_on_back(0, true) {
            return Ok(false);
        }

        self.config.guide_topic = Some(GuideTopic::Menu);

        loop {
            match self.resolve_guide_topic_prompt() {
                Ok(()) => break,
                Err(SetupError::Io(error)) if is_back_navigation(&error) => {
                    self.config.guide_topic = None;
                    match self.resolve_setup_entry_prompt(self.resolved_ui_mode) {
                        Ok(()) => {
                            if self.config.guide_topic == Some(GuideTopic::Menu) {
                                continue;
                            }
                            break;
                        }
                        Err(SetupError::Io(error)) if is_back_navigation(&error) => {
                            self.config.guide_topic = original_guide_topic;
                            return Ok(false);
                        }
                        Err(error) => {
                            self.config.guide_topic = original_guide_topic;
                            return Err(error);
                        }
                    }
                }
                Err(error) => {
                    self.config.guide_topic = original_guide_topic;
                    return Err(error);
                }
            }
        }

        self.apply_run_mode_defaults().await?;
        self.reset_plan_state();
        Ok(true)
    }

    /// Re-show the Quick vs. Advanced entry prompt when the user presses
    /// Ctrl+B on the very first plan step in quick-setup mode.
    async fn reroute_tui_back_from_first_quick_step(&mut self) -> Result<bool, SetupError> {
        if self.is_guide_mode() || !self.should_reopen_tui_menus_on_back(0, true) {
            return Ok(false);
        }

        // Re-present the "Quick Setup vs. Advanced Setup" entry prompt.
        // If the user presses Ctrl+B again at that prompt, treat it as a
        // no-op (stay on current plan) since there is nothing further back.
        match self.resolve_setup_entry_prompt(self.resolved_ui_mode) {
            Ok(()) => {
                // User may have switched to Advanced, which sets guide_topic.
                // If they picked Advanced, show the topic menu too.
                if self.config.guide_topic == Some(GuideTopic::Menu) {
                    self.resolve_guide_topic_prompt()?;
                }
            }
            Err(SetupError::Io(error)) if is_back_navigation(&error) => {
                // Nowhere further back to go — stay on the current plan.
                return Ok(false);
            }
            Err(error) => return Err(error),
        }

        self.apply_run_mode_defaults().await?;
        self.reset_plan_state();
        Ok(true)
    }

    fn should_persist_step(&self, step_id: WizardStepId) -> bool {
        !matches!(
            step_id,
            WizardStepId::Profile | WizardStepId::ChannelContinuity | WizardStepId::Summary
        )
    }

    async fn execute_step(&mut self, step_id: WizardStepId) -> Result<StepStatus, SetupError> {
        match step_id {
            WizardStepId::CliSkin => {
                self.step_cli_skin()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Profile => {
                self.step_profile()?;
                self.apply_profile_defaults();
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
                if self.is_quick_setup() {
                    self.apply_quick_embeddings_defaults();
                    self.step_smart_routing().await?;
                }
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
                if self.is_quick_setup() {
                    self.step_primary_channel_quick().await?;
                } else {
                    self.step_channels().await?;
                }
                Ok(StepStatus::Completed)
            }
            WizardStepId::ChannelContinuity => {
                self.step_channel_continuity()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::ChannelVerification => {
                let issues = self.step_channel_verification().await?;
                if self.is_quick_setup() {
                    self.apply_quick_notification_defaults();
                }
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
            WizardStepId::CodingWorkers => {
                self.step_coding_workers().await?;
                Ok(
                    if self.settings.claude_code_enabled || self.settings.codex_code_enabled {
                        StepStatus::Completed
                    } else {
                        StepStatus::Skipped
                    },
                )
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

    fn resolve_guide_topic_prompt(&mut self) -> Result<(), SetupError> {
        if self.config.guide_topic != Some(GuideTopic::Menu) {
            return Ok(());
        }

        print_info("Choose the settings topic you want to revisit.");
        let options = [
            "AI & Models",
            "Channels & Notifications",
            "Agent & Experience",
            "Tools & Safety",
            "Automation & Skills",
            "Runtime & Diagnostics",
        ];
        let choice = select_one("Guided settings topic", &options).map_err(SetupError::Io)?;
        self.config.guide_topic = Some(match choice {
            1 => GuideTopic::Channels,
            2 => GuideTopic::Agent,
            3 => GuideTopic::Tools,
            4 => GuideTopic::Automation,
            5 => GuideTopic::Runtime,
            _ => GuideTopic::Ai,
        });
        Ok(())
    }

    fn resolve_setup_entry_prompt(&mut self, ui_mode: UiMode) -> Result<(), SetupError> {
        if !matches!(ui_mode, UiMode::Cli | UiMode::Tui)
            || self.config.channels_only
            || self.config.guide_topic.is_some()
            || self.config.profile.is_some()
        {
            return Ok(());
        }

        print_info(
            "Choose whether you want the streamlined default path or the guided advanced lane.",
        );
        let options = [
            "Quick Setup     - reduced day-one path that gets ThinClaw running fast",
            "Advanced Setup  - choose a topic and tune more deeply before launch",
        ];
        let choice = select_one("Setup mode", &options).map_err(SetupError::Io)?;
        if choice == 1 {
            self.config.guide_topic = Some(GuideTopic::Menu);
            print_info(
                "Advanced Setup keeps Quick Setup short and opens the guided topic menu instead.",
            );
        } else {
            print_success("Quick Setup selected. ThinClaw will stay on the reduced day-one path.");
        }

        Ok(())
    }

    async fn prepare_run_mode(&mut self, ui_mode: UiMode) -> Result<(), SetupError> {
        self.resolve_setup_entry_prompt(ui_mode)?;
        self.resolve_guide_topic_prompt()?;
        self.apply_run_mode_defaults().await?;

        Ok(())
    }

    async fn apply_run_mode_defaults(&mut self) -> Result<(), SetupError> {
        if let Some(profile) = self.config.profile {
            self.selected_profile = profile;
            self.apply_profile_defaults();
        }

        if self.is_quick_setup() {
            self.auto_configure_quick_runtime_defaults().await?;
        } else if self.config.channels_only || self.is_guide_mode() {
            let _ = self.reconnect_existing_db().await;
        }

        Ok(())
    }

    /// Run the setup wizard.
    ///
    /// Settings are persisted incrementally after each successful step so
    /// that progress is not lost if a later step fails. On re-run, existing
    /// settings are loaded from the database after Step 1 establishes a
    /// connection, so users don't have to re-enter everything.
    pub async fn run(&mut self) -> Result<(), SetupError> {
        let requested_ui_mode = self.config.ui_mode;
        let ui_mode = self.resolve_ui_mode();
        self.resolved_ui_mode = ui_mode;

        let prompt_mode = match ui_mode {
            UiMode::Tui => PromptRenderMode::Tui,
            UiMode::Cli | UiMode::Auto => PromptRenderMode::Cli,
        };
        {
            let _prompt_mode = push_prompt_ui_mode(prompt_mode);
            self.prepare_run_mode(ui_mode).await?;
        }
        self.reset_plan_state();

        match (requested_ui_mode, ui_mode) {
            (_, UiMode::Cli) => self.run_cli_flow().await,
            (UiMode::Auto, UiMode::Tui) => match self.run_tui_flow().await {
                Ok(()) => Ok(()),
                Err(SetupError::Cancelled) => Err(SetupError::Cancelled),
                Err(error) => {
                    print_warning(&format!(
                        "The onboarding TUI could not continue cleanly ({}). Falling back to the terminal wizard.",
                        error
                    ));
                    self.resolved_ui_mode = UiMode::Cli;
                    self.reset_plan_state();
                    self.run_cli_flow().await
                }
            },
            (_, UiMode::Tui) => self.run_tui_flow().await,
            (_, UiMode::Auto) => unreachable!("auto mode is resolved before onboarding begins"),
        }
    }
}
