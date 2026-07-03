//! Onboarding profile selection and profile-driven default application,
//! including headless/remote service defaults and remote-provider followups.

use crate::settings::{OnboardingFollowupCategory, OnboardingFollowupStatus};
use crate::setup::prompts::{print_info, print_success, select_one};

use super::{FollowupDraft, OnboardingProfile, SetupError, SetupWizard};

impl SetupWizard {
    pub(super) fn step_profile(&mut self) -> Result<(), SetupError> {
        if let Some(profile) = self.config.profile {
            self.selected_profile = profile;
            print_success(&format!(
                "Using the {} profile from --profile.",
                self.selected_profile.title()
            ));
            print_info(self.selected_profile.description());
            return Ok(());
        }

        let options = [
            "Balanced            - calm defaults for most first runs",
            "Local & Private     - prefer local models and fewer external services",
            "Builder & Coding    - bias for tools, coding, and stronger routing",
            "Channel-First       - prioritize reachability and notification setup",
            "Remote / SSH Host   - safe service runtime with WebUI access via SSH tunnel",
            "Pi OS Lite 64-bit   - Raspberry Pi headless remote service, no desktop actions",
            "Custom / Advanced   - start neutral and tune each major choice directly",
        ];
        print_info("Choose the lane that best matches the system you want to leave setup with.");
        let choice = select_one("Choose your setup lane", &options).map_err(SetupError::Io)?;
        self.selected_profile = match choice {
            1 => OnboardingProfile::LocalAndPrivate,
            2 => OnboardingProfile::BuilderAndCoding,
            3 => OnboardingProfile::ChannelFirst,
            4 => OnboardingProfile::RemoteServer,
            5 => OnboardingProfile::PiOsLite64,
            6 => OnboardingProfile::CustomAdvanced,
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

    pub(super) fn apply_profile_defaults(&mut self) {
        match self.selected_profile {
            OnboardingProfile::Balanced => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "log".to_string();
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
                self.settings.observability_backend = "log".to_string();
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
                self.settings.observability_backend = "log".to_string();
                self.settings.providers.smart_routing_enabled = true;
                self.settings.providers.routing_mode =
                    crate::settings::RoutingMode::AdvisorExecutor;
                self.settings.providers.advisor_max_calls =
                    self.settings.providers.advisor_max_calls.max(4);
                self.settings.routines_enabled = true;
                self.settings.heartbeat.enabled = false;
            }
            OnboardingProfile::ChannelFirst => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "log".to_string();
                self.settings.providers.smart_routing_enabled = true;
                if self.settings.providers.routing_mode == crate::settings::RoutingMode::PrimaryOnly
                {
                    self.settings.providers.routing_mode = crate::settings::RoutingMode::CheapSplit;
                }
                self.settings.routines_enabled = true;
            }
            OnboardingProfile::RemoteServer => self.apply_headless_remote_profile_defaults(false),
            OnboardingProfile::PiOsLite64 => self.apply_headless_remote_profile_defaults(true),
            OnboardingProfile::CustomAdvanced => {}
        }
    }

    fn apply_headless_remote_profile_defaults(&mut self, pi_os_lite: bool) {
        self.settings.skills_enabled = true;
        self.settings.observability_backend = "log".to_string();
        self.settings.providers.smart_routing_enabled = true;
        if self.settings.providers.routing_mode == crate::settings::RoutingMode::PrimaryOnly {
            self.settings.providers.routing_mode = crate::settings::RoutingMode::CheapSplit;
        }
        self.settings.routines_enabled = true;
        self.settings.heartbeat.enabled = false;
        self.settings.channels.cli_enabled = Some(false);
        self.settings.channels.gateway_enabled = Some(true);
        let gateway_host = self.remote_gateway_host_or_loopback().to_string();
        self.settings.channels.gateway_host = Some(gateway_host);
        self.settings.channels.gateway_port =
            Some(self.settings.channels.gateway_port.unwrap_or(3000));
        self.ensure_gateway_auth_token();
        if self.settings.database_backend.is_none() {
            self.settings.database_backend = Some("libsql".to_string());
        }
        if self.settings.libsql_path.is_none() {
            self.settings.libsql_path = Some(
                crate::config::default_libsql_path()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        if self.settings.secrets_master_key_source == crate::settings::KeySource::Env {
            self.settings.secrets.allow_env_master_key = true;
            self.settings.secrets.master_key_source = crate::settings::SecretsMasterKeySource::Env;
        }
        if pi_os_lite {
            self.settings.desktop_autonomy.enabled = false;
            self.settings.desktop_autonomy.profile = crate::settings::DesktopAutonomyProfile::Off;
        }
    }

    pub(super) fn ensure_gateway_auth_token(&mut self) {
        let has_token = self
            .settings
            .channels
            .gateway_auth_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty());
        if has_token {
            return;
        }

        use rand::Rng;
        let token: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(48)
            .map(char::from)
            .collect();
        self.settings.channels.gateway_auth_token = Some(token);
    }

    fn remote_gateway_host_or_loopback(&self) -> &str {
        self.settings
            .channels
            .gateway_host
            .as_deref()
            .filter(|host| !host.trim().is_empty())
            .unwrap_or("127.0.0.1")
    }

    pub(super) fn ensure_remote_provider_followup(&mut self) {
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
}
