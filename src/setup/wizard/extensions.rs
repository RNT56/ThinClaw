//! Extensions wizard step: tool installation from registry.

use crate::setup::prompts::{print_info, print_success, select_many, select_one};

use super::helpers::{discover_installed_tools, load_registry_catalog};
use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) async fn step_extensions(&mut self) -> Result<(), SetupError> {
        let catalog = match load_registry_catalog() {
            Some(c) => c,
            None => {
                print_info("Extension registry not found. Tool installation will be skipped.");
                print_info("Install tools manually with: thinclaw extensions tools install <path>");
                return Ok(());
            }
        };

        let tools: Vec<_> = catalog
            .list(Some(crate::registry::manifest::ManifestKind::Tool), None)
            .into_iter()
            .cloned()
            .collect();

        if tools.is_empty() {
            print_info("No installable tools were found in the registry.");
            return Ok(());
        }

        print_info("Tools available from the registry:");
        print_info("Pick the tools to install now. You can add more later with:");
        print_info("  thinclaw extensions registry install <name>");
        crate::setup::prompts::print_blank_line();

        let default_bundle = match self.selected_profile {
            super::OnboardingProfile::LocalAndPrivate => 0,
            super::OnboardingProfile::BuilderAndCoding => 2,
            super::OnboardingProfile::Balanced
            | super::OnboardingProfile::ChannelFirst
            | super::OnboardingProfile::RemoteServer
            | super::OnboardingProfile::PiOsLite64 => 1,
            super::OnboardingProfile::CustomAdvanced => 0,
        };
        let bundle_options = [
            "Safe      - installed tools plus low-friction defaults",
            "Balanced  - installed tools plus registry defaults (recommended)",
            "Power     - preselect every available tool for review",
        ];
        let bundle_choice = select_one("Tool bundle", &bundle_options).map_err(SetupError::Io)?;
        let bundle_choice = if bundle_choice < bundle_options.len() {
            bundle_choice
        } else {
            default_bundle
        };

        // Check which tools are already installed
        let tools_dir = dirs::home_dir()
            .ok_or_else(|| SetupError::Config("Could not determine home directory".into()))?
            .join(".thinclaw/tools");

        let installed_tools = discover_installed_tools(&tools_dir).await;

        // Build options: show display_name + description, pre-check "default" tagged + already installed
        let mut options: Vec<(String, bool)> = Vec::new();
        for tool in &tools {
            let is_installed = installed_tools.contains(&tool.name);
            let is_default = tool.tags.contains(&"default".to_string());
            let no_auth = tool
                .auth_summary
                .as_ref()
                .and_then(|a| a.method.as_deref())
                .is_none_or(|method| method == "none");
            let status = if is_installed { " (installed)" } else { "" };
            let auth_hint = tool
                .auth_summary
                .as_ref()
                .and_then(|a| a.method.as_deref())
                .map(|m| format!(" [{}]", m))
                .unwrap_or_default();

            let label = format!(
                "{}{}{} - {}",
                tool.display_name, auth_hint, status, tool.description
            );
            let preselected = match bundle_choice {
                0 => is_installed || (is_default && no_auth),
                2 => true,
                _ => is_default || is_installed,
            };
            options.push((label, preselected));
        }

        let options_refs: Vec<(&str, bool)> =
            options.iter().map(|(s, b)| (s.as_str(), *b)).collect();

        let selected = select_many("Which tools do you want to install?", &options_refs)
            .map_err(SetupError::Io)?;

        if selected.is_empty() {
            print_info("No tools selected. Skipping install.");
            return Ok(());
        }

        // Selection is side-effect-free. The final Apply transaction owns all
        // extraction, downloads, builds, and publication.
        let mut planned_count = 0;
        let mut auth_needed: Vec<String> = Vec::new();

        for idx in &selected {
            let tool = &tools[*idx];
            if installed_tools.contains(&tool.name) {
                continue; // Already installed, skip
            }

            self.queue_action(super::PendingSetupAction::InstallTool {
                name: tool.name.clone(),
            });
            planned_count += 1;

            if let Some(auth) = &tool.auth_summary
                && auth.method.as_deref() != Some("none")
                && auth.method.is_some()
            {
                let provider = auth.provider.as_deref().unwrap_or(&tool.name);
                let hint = format!(
                    "  {} - thinclaw extensions tools auth {}",
                    provider, tool.name
                );
                if !auth_needed
                    .iter()
                    .any(|h| h.starts_with(&format!("  {} -", provider)))
                {
                    auth_needed.push(hint);
                }
            }
        }

        if planned_count > 0 {
            crate::setup::prompts::print_blank_line();
            print_success(&format!(
                "{} tool installation(s) added to the Apply plan.",
                planned_count
            ));
        }

        if !auth_needed.is_empty() {
            crate::setup::prompts::print_blank_line();
            print_info("Some tools still need authentication. Run these after setup:");
            for hint in &auth_needed {
                print_info(hint);
            }
        }

        Ok(())
    }
}
