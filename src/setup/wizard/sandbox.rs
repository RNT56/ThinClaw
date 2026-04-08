//! Sandbox wizard steps: Docker sandbox and Claude Code.

use secrecy::SecretString;

use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_info, print_success,
};

use super::{SetupError, SetupWizard};

const ANTHROPIC_PROVIDER_SECRET_NAME: &str = "llm_anthropic_api_key";

impl SetupWizard {
    pub(super) fn enable_anthropic_provider_for_claude_code_key(&mut self) {
        self.ensure_provider_enabled("anthropic");
        self.ensure_provider_slot_defaults("anthropic");
    }

    async fn maybe_link_claude_code_api_key_to_anthropic_provider(
        &mut self,
        api_key: &str,
    ) -> Result<(), SetupError> {
        let trimmed = api_key.trim();
        if trimmed.is_empty() || self.has_saved_secret(ANTHROPIC_PROVIDER_SECRET_NAME).await {
            return Ok(());
        }

        println!();
        print_info(
            "ThinClaw can also reuse this Anthropic API key for the general Anthropic provider.",
        );
        print_info(
            "That makes it available in Provider Vault, the WebUI, and normal Anthropic model routing.",
        );
        println!();

        if !confirm(
            "Also save this API key for ThinClaw's Anthropic provider and WebUI?",
            true,
        )
        .map_err(SetupError::Io)?
        {
            return Ok(());
        }

        let Ok(ctx) = self.init_secrets_context().await else {
            print_info(
                "Secrets store not available. Claude Code will still use the key, but Anthropic provider reuse was not persisted.",
            );
            return Ok(());
        };

        if let Err(e) = ctx
            .save_secret(
                ANTHROPIC_PROVIDER_SECRET_NAME,
                &SecretString::from(trimmed.to_string()),
            )
            .await
        {
            print_error(&format!(
                "Failed to save the shared Anthropic provider key: {}",
                e
            ));
            print_info("Claude Code will keep using its own key source.");
            return Ok(());
        }

        self.enable_anthropic_provider_for_claude_code_key();
        print_success("Anthropic provider credentials saved for general use.");
        Ok(())
    }

    pub(super) async fn step_docker_sandbox(&mut self) -> Result<(), SetupError> {
        // ── Part A: Local tools for the main agent ───────────────────────
        println!();
        print_info("═══ Main Agent: Local Tools ═══");
        println!();
        print_info("ThinClaw's main agent always runs natively on your machine.");
        print_info("Enabling local tools gives the agent full access to:");
        print_info("  • Shell commands (run scripts, install packages, etc.)");
        print_info("  • File read/write anywhere on disk");
        print_info("  • Screen capture (if enabled separately)");
        println!();
        print_info("Without local tools, the agent can only use web search, memory,");
        print_info("and WASM-sandboxed extensions. No direct host access.");
        println!();

        let allow_local = confirm("Allow ThinClaw to use local tools on your machine?", false)
            .map_err(SetupError::Io)?;
        self.settings.agent.allow_local_tools = allow_local;

        if allow_local {
            print_success("Local tools enabled. The agent can run commands and access files.");
            print_info("You can disable this later with ALLOW_LOCAL_TOOLS=false.");
        } else {
            print_info("Local tools disabled. The agent will use sandboxed tools only.");
            print_info("Enable later with ALLOW_LOCAL_TOOLS=true.");
        }

        // ── Part B: Docker sandbox for worker processes ──────────────────
        println!();
        print_info("═══ Docker Sandbox (Worker Processes) ═══");
        println!();
        print_info("Docker sandboxing is separate from local tools above.");
        print_info("It isolates *worker processes* like Claude Code — they run inside");
        print_info("Docker containers with no access to your credentials or full filesystem.");
        println!();
        print_info("This does NOT affect ThinClaw's main agent. The main agent always");
        print_info("runs natively, governed by the 'local tools' setting above.");
        println!();
        print_info("Docker is required for: Claude Code sandbox, container-based builds.");
        println!();

        if !confirm("Enable Docker sandbox for worker processes?", false).map_err(SetupError::Io)? {
            self.settings.sandbox.enabled = false;
            print_info("Docker sandbox disabled. Worker processes will not use containers.");
            print_info("You can enable it later with SANDBOX_ENABLED=true.");
            return Ok(());
        }

        // Check Docker availability
        let detection = crate::sandbox::detect::check_docker().await;

        match detection.status {
            crate::sandbox::detect::DockerStatus::Available => {
                self.settings.sandbox.enabled = true;
                print_success("Docker is installed and running. Worker sandbox enabled.");
            }
            crate::sandbox::detect::DockerStatus::NotInstalled
            | crate::sandbox::detect::DockerStatus::NotRunning => {
                println!();
                let not_installed =
                    detection.status == crate::sandbox::detect::DockerStatus::NotInstalled;
                if not_installed {
                    print_error("Docker is not installed.");
                    print_info(detection.platform.install_hint());
                } else {
                    print_error("Docker is installed but not running.");
                    print_info(detection.platform.start_hint());
                }
                println!();

                let retry_prompt = if not_installed {
                    "Retry after installing Docker?"
                } else {
                    "Retry after starting Docker?"
                };
                if confirm(retry_prompt, false).map_err(SetupError::Io)? {
                    let retry = crate::sandbox::detect::check_docker().await;
                    if retry.status.is_ok() {
                        self.settings.sandbox.enabled = true;
                        print_success(if not_installed {
                            "Docker is now available. Worker sandbox enabled."
                        } else {
                            "Docker is now running. Worker sandbox enabled."
                        });
                    } else {
                        self.settings.sandbox.enabled = false;
                        print_info(if not_installed {
                            "Docker still not available. Worker sandbox disabled for now."
                        } else {
                            "Docker still not responding. Worker sandbox disabled for now."
                        });
                    }
                } else {
                    self.settings.sandbox.enabled = false;
                    print_info(if not_installed {
                        "Worker sandbox disabled. Install Docker and set SANDBOX_ENABLED=true later."
                    } else {
                        "Worker sandbox disabled. Start Docker and set SANDBOX_ENABLED=true later."
                    });
                }
            }
            crate::sandbox::detect::DockerStatus::Disabled => {
                self.settings.sandbox.enabled = false;
            }
        }

        // ── Part C: Build worker image if needed ─────────────────────────
        if self.settings.sandbox.enabled {
            println!();
            print_info("═══ Worker Docker Image ═══");
            println!();

            // Check if the image already exists
            let image_name = &self.settings.sandbox.image;
            let image_exists = std::process::Command::new("docker")
                .args(["image", "inspect", image_name])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if image_exists {
                print_success(&format!("Worker image '{}' already exists.", image_name));
            } else {
                print_info(&format!("Worker image '{}' not found locally.", image_name));
                print_info("This image is required for Docker sandbox and Claude Code jobs.");
                print_info("Building it now takes 5-15 minutes (one-time).");
                println!();

                if confirm("Build the worker image now?", true).map_err(SetupError::Io)? {
                    print_info("Building thinclaw-worker image (this may take a while)...");

                    // Find the repo root (where Dockerfile.worker lives)
                    let repo_root =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

                    let status = std::process::Command::new("docker")
                        .args(["build", "-f", "Dockerfile.worker", "-t", image_name, "."])
                        .current_dir(&repo_root)
                        .stdin(std::process::Stdio::inherit())
                        .stdout(std::process::Stdio::inherit())
                        .stderr(std::process::Stdio::inherit())
                        .status();

                    match status {
                        Ok(s) if s.success() => {
                            print_success("Worker image built successfully.");
                        }
                        Ok(s) => {
                            print_error(&format!(
                                "Docker build failed (exit code {:?}).",
                                s.code()
                            ));
                            print_info("You can build it later with:");
                            print_info("  docker build -f Dockerfile.worker -t thinclaw-worker .");
                        }
                        Err(e) => {
                            print_error(&format!("Failed to start docker build: {}", e));
                            print_info("You can build it later with:");
                            print_info("  docker build -f Dockerfile.worker -t thinclaw-worker .");
                        }
                    }
                } else {
                    print_info("Skipping image build. Build it later with:");
                    print_info("  docker build -f Dockerfile.worker -t thinclaw-worker .");
                }
            }
        }

        Ok(())
    }

    /// Step 7: Agent identity (name).
    pub(super) async fn step_claude_code(&mut self) -> Result<(), SetupError> {
        // Claude Code requires the Docker sandbox to be enabled
        if !self.settings.sandbox.enabled {
            print_info("Claude Code requires Docker sandbox (not enabled in step 10).");
            print_info("Skipping Claude Code configuration.");
            self.settings.claude_code_enabled = false;
            return Ok(());
        }

        print_info("Claude Code sandbox allows ThinClaw to delegate complex coding");
        print_info("tasks to Anthropic's Claude Code CLI running inside a Docker container.");
        print_info("Requires an Anthropic API key or Claude Code OAuth session.");
        println!();

        if !confirm("Enable Claude Code sandbox?", false).map_err(SetupError::Io)? {
            self.settings.claude_code_enabled = false;
            print_info("Claude Code disabled. Enable later with CLAUDE_CODE_ENABLED=true.");
            return Ok(());
        }

        self.settings.claude_code_enabled = true;

        // ── Auth strategy ────────────────────────────────────────────────
        println!();
        print_info("═══ Claude Code Authentication ═══");
        println!();

        // Check existing auth sources
        let env_api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let has_oauth = crate::config::ClaudeCodeConfig::extract_oauth_token().is_some();

        let keychain_api_key = crate::secrets::keychain::get_api_key(
            crate::secrets::keychain::CLAUDE_CODE_API_KEY_ACCOUNT,
        )
        .await
        .filter(|value| !value.trim().is_empty());

        if let Some(api_key) = env_api_key.as_deref() {
            print_success("✓ ANTHROPIC_API_KEY found in environment. This will be used.");
            self.maybe_link_claude_code_api_key_to_anthropic_provider(api_key)
                .await?;
        } else if let Some(api_key) = keychain_api_key.as_deref() {
            print_success("✓ Anthropic API key found in OS keychain. This will be used.");
            self.maybe_link_claude_code_api_key_to_anthropic_provider(api_key)
                .await?;
        } else if has_oauth {
            print_success("✓ Claude Code OAuth session found. This will be used.");
            print_info("  (Token from 'claude login' — typically valid for 8-12 hours)");
        } else {
            print_info("No existing auth found. Claude Code containers need one of:");
            print_info("  1. Anthropic API key (stored securely in OS keychain)");
            print_info("  2. OAuth session from 'claude login' on this machine");
            println!();

            if confirm(
                "Enter an Anthropic API key to store in the OS keychain?",
                true,
            )
            .map_err(SetupError::Io)?
            {
                let api_key = input("Anthropic API key (sk-ant-...)").map_err(SetupError::Io)?;

                if api_key.starts_with("sk-ant-") {
                    match crate::secrets::keychain::store_api_key(
                        crate::secrets::keychain::CLAUDE_CODE_API_KEY_ACCOUNT,
                        &api_key,
                    )
                    .await
                    {
                        Ok(()) => {
                            print_success("API key stored securely in OS keychain.");
                            print_info(
                                "It will be injected into Claude Code containers at runtime.",
                            );
                            self.maybe_link_claude_code_api_key_to_anthropic_provider(&api_key)
                                .await?;
                        }
                        Err(e) => {
                            print_error(&format!("Failed to store in keychain: {}", e));
                            print_info(
                                "You can set ANTHROPIC_API_KEY in your environment instead.",
                            );
                        }
                    }
                } else {
                    print_error("Key doesn't look like an Anthropic API key (expected sk-ant-...)");
                    print_info("You can set ANTHROPIC_API_KEY in your environment later.");
                }
            } else {
                print_info("No API key stored. You can:");
                print_info("  • Run 'claude login' to set up OAuth");
                print_info("  • Set ANTHROPIC_API_KEY in your environment");
            }
        }

        // ── Model ────────────────────────────────────────────────────────
        println!();
        let model =
            optional_input("Claude Code model", Some("default: sonnet")).map_err(SetupError::Io)?;
        if let Some(m) = model {
            if !m.is_empty() {
                self.settings.claude_code_model = Some(m);
            }
        }

        // Max turns
        let turns =
            optional_input("Max agentic turns", Some("default: 50")).map_err(SetupError::Io)?;
        if let Some(t) = turns {
            if let Ok(n) = t.parse::<u32>() {
                self.settings.claude_code_max_turns = Some(n);
            }
        }

        let model_display = self
            .settings
            .claude_code_model
            .as_deref()
            .unwrap_or("sonnet");
        let turns_display = self.settings.claude_code_max_turns.unwrap_or(50);
        print_success(&format!(
            "Claude Code enabled (model: {}, max turns: {})",
            model_display, turns_display
        ));

        Ok(())
    }
}
