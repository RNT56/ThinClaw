//! Sandbox wizard steps: Docker sandbox and container coding agents.

use secrecy::SecretString;

use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_info, print_success, select_many,
};

use super::{SetupError, SetupWizard};

const ANTHROPIC_PROVIDER_SECRET_NAME: &str = "llm_anthropic_api_key";
const OPENAI_PROVIDER_SECRET_NAME: &str = "llm_openai_api_key";

impl SetupWizard {
    pub(super) fn enable_anthropic_provider_for_claude_code_key(&mut self) {
        self.ensure_provider_enabled("anthropic");
        self.ensure_provider_slot_defaults("anthropic");
    }

    pub(super) fn enable_openai_provider_for_codex_code_key(&mut self) {
        self.ensure_provider_enabled("openai");
        self.ensure_provider_slot_defaults("openai");
    }

    async fn maybe_link_claude_code_api_key_to_anthropic_provider(
        &mut self,
        api_key: &str,
    ) -> Result<(), SetupError> {
        let trimmed = api_key.trim();
        if trimmed.is_empty() || self.has_saved_secret(ANTHROPIC_PROVIDER_SECRET_NAME).await {
            return Ok(());
        }

        crate::setup::prompts::print_blank_line();
        print_info(
            "ThinClaw can also reuse this Anthropic API key for the general Anthropic provider.",
        );
        print_info("That makes it available in Provider Vault, the Web UI, and Anthropic routing.");
        crate::setup::prompts::print_blank_line();

        if !confirm(
            "Also save this API key for ThinClaw's Anthropic provider and Web UI?",
            true,
        )
        .map_err(SetupError::Io)?
        {
            return Ok(());
        }

        let Ok(ctx) = self.init_secrets_context().await else {
            print_info(
                "Secrets store not available. Claude Code will still use the key, but the shared Anthropic provider key was not saved.",
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
        print_success("Anthropic provider credentials saved for reuse.");
        Ok(())
    }

    async fn maybe_link_codex_code_api_key_to_openai_provider(
        &mut self,
        api_key: &str,
    ) -> Result<(), SetupError> {
        let trimmed = api_key.trim();
        if trimmed.is_empty() || self.has_saved_secret(OPENAI_PROVIDER_SECRET_NAME).await {
            return Ok(());
        }

        crate::setup::prompts::print_blank_line();
        print_info("ThinClaw can also reuse this OpenAI API key for the general OpenAI provider.");
        print_info("That makes it available in Provider Vault, the Web UI, and OpenAI routing.");
        crate::setup::prompts::print_blank_line();

        if !confirm(
            "Also save this API key for ThinClaw's OpenAI provider and Web UI?",
            true,
        )
        .map_err(SetupError::Io)?
        {
            return Ok(());
        }

        let Ok(ctx) = self.init_secrets_context().await else {
            print_info(
                "Secrets store not available. Codex will still use the key, but the shared OpenAI provider key was not saved.",
            );
            return Ok(());
        };

        if let Err(e) = ctx
            .save_secret(
                OPENAI_PROVIDER_SECRET_NAME,
                &SecretString::from(trimmed.to_string()),
            )
            .await
        {
            print_error(&format!(
                "Failed to save the shared OpenAI provider key: {}",
                e
            ));
            print_info("Codex will keep using its own key source.");
            return Ok(());
        }

        self.enable_openai_provider_for_codex_code_key();
        print_success("OpenAI provider credentials saved for reuse.");
        Ok(())
    }

    pub(super) async fn step_docker_sandbox(&mut self) -> Result<(), SetupError> {
        crate::setup::prompts::print_blank_line();
        print_info("═══ Worker Sandbox ═══");
        crate::setup::prompts::print_blank_line();
        print_info("The worker sandbox isolates delegated workers like Claude Code and Codex.");
        print_info(
            "It does not change the autonomy level you chose for the main interactive agent.",
        );
        crate::setup::prompts::print_blank_line();
        print_info("Docker is required for container coding workers and isolated worker builds.");
        crate::setup::prompts::print_blank_line();

        if !confirm("Enable worker sandbox?", true).map_err(SetupError::Io)? {
            self.settings.sandbox.enabled = false;
            print_info("Docker sandbox disabled. Worker processes will run without containers.");
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
                crate::setup::prompts::print_blank_line();
                let not_installed =
                    detection.status == crate::sandbox::detect::DockerStatus::NotInstalled;
                if not_installed {
                    print_error("Docker is not installed.");
                    print_info(detection.platform.install_hint());
                } else {
                    print_error("Docker is installed but not running.");
                    print_info(detection.platform.start_hint());
                }
                crate::setup::prompts::print_blank_line();

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
                            "Docker still isn't available. Worker sandbox is disabled for now."
                        } else {
                            "Docker still isn't responding. Worker sandbox is disabled for now."
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
            crate::setup::prompts::print_blank_line();
            print_info("═══ Worker Docker Image ═══");
            crate::setup::prompts::print_blank_line();

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
                print_info(&format!(
                    "Worker image '{}' wasn't found locally.",
                    image_name
                ));
                print_info(
                    "This image is required for the Docker sandbox and container coding agent jobs.",
                );
                print_info("Building it now usually takes 5-15 minutes, one time.");
                crate::setup::prompts::print_blank_line();

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

    pub(super) async fn step_coding_workers(&mut self) -> Result<(), SetupError> {
        if !self.settings.sandbox.enabled {
            self.settings.claude_code_enabled = false;
            self.settings.codex_code_enabled = false;
            print_info("Worker sandbox is disabled. Skipping coding worker selection.");
            return Ok(());
        }

        print_info("Coding workers are optional delegates for code-heavy tasks.");
        print_info("You can leave them off and enable them later from guided settings.");
        crate::setup::prompts::print_blank_line();

        if !confirm("Enable coding workers now?", false).map_err(SetupError::Io)? {
            self.settings.claude_code_enabled = false;
            self.settings.codex_code_enabled = false;
            print_info("Coding workers skipped for now.");
            return Ok(());
        }

        if self.is_quick_setup() {
            let options = [("Claude Code Worker", false), ("Codex Worker", false)];
            let selected =
                select_many("Select coding workers to enable", &options).map_err(SetupError::Io)?;

            let enable_claude = selected.contains(&0);
            let enable_codex = selected.contains(&1);
            self.settings.claude_code_enabled = enable_claude;
            self.settings.codex_code_enabled = enable_codex;

            if !enable_claude && !enable_codex {
                print_info("No coding workers selected. You can enable them later from settings.");
                return Ok(());
            }

            if enable_claude {
                crate::setup::prompts::print_blank_line();
                self.step_claude_code_inner(false).await?;
            }
            if enable_codex {
                crate::setup::prompts::print_blank_line();
                self.step_codex_code_inner(false).await?;
            }
            return Ok(());
        }

        crate::setup::prompts::print_blank_line();
        self.step_claude_code().await?;
        crate::setup::prompts::print_blank_line();
        self.step_codex_code().await?;
        Ok(())
    }

    /// Step 7: Agent identity (name).
    async fn step_claude_code_inner(&mut self, prompt_enable: bool) -> Result<(), SetupError> {
        // Claude Code requires the Docker sandbox to be enabled
        if !self.settings.sandbox.enabled {
            print_info("Claude Code requires the Docker sandbox, which is not enabled yet.");
            print_info("Skipping Claude Code setup.");
            self.settings.claude_code_enabled = false;
            return Ok(());
        }

        print_info("Claude Code sandbox lets ThinClaw delegate complex coding tasks");
        print_info("to Anthropic's Claude Code CLI running inside a Docker container.");
        print_info("It requires either an Anthropic API key or a Claude Code OAuth session.");
        crate::setup::prompts::print_blank_line();

        if prompt_enable
            && !confirm("Enable Claude Code sandbox?", false).map_err(SetupError::Io)?
        {
            self.settings.claude_code_enabled = false;
            print_info(
                "Claude Code disabled. You can turn it on later with CLAUDE_CODE_ENABLED=true.",
            );
            return Ok(());
        }

        self.settings.claude_code_enabled = true;

        // ── Auth strategy ────────────────────────────────────────────────
        crate::setup::prompts::print_blank_line();
        print_info("═══ Claude Code Authentication ═══");
        crate::setup::prompts::print_blank_line();

        // Check existing auth sources
        let api_key_storage_label = if cfg!(target_os = "macos") {
            format!(
                "ThinClaw's encrypted secrets store (protected by {})",
                crate::platform::secure_store::display_name()
            )
        } else {
            crate::platform::secure_store::display_name().to_string()
        };
        let env_api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let has_oauth = crate::config::ClaudeCodeConfig::extract_oauth_token().is_some();
        let saved_api_key = self
            .resolve_provider_secret_value("ANTHROPIC_API_KEY", ANTHROPIC_PROVIDER_SECRET_NAME)
            .await
            .filter(|value| !value.trim().is_empty());
        let keychain_api_key = if cfg!(target_os = "macos") && saved_api_key.is_some() {
            None
        } else {
            crate::platform::secure_store::get_api_key(
                crate::platform::secure_store::CLAUDE_CODE_API_KEY_ACCOUNT,
            )
            .await
            .filter(|value| !value.trim().is_empty())
        };

        if let Some(api_key) = env_api_key.as_deref() {
            print_success("✓ ANTHROPIC_API_KEY found in the environment. Using it now.");
            self.maybe_link_claude_code_api_key_to_anthropic_provider(api_key)
                .await?;
        } else if saved_api_key.is_some() {
            print_success(&format!(
                "✓ Anthropic API key found in {}. Using it now.",
                api_key_storage_label
            ));
            self.enable_anthropic_provider_for_claude_code_key();
        } else if let Some(api_key) = keychain_api_key.as_deref() {
            print_success(&format!(
                "✓ Anthropic API key found in the {}. Using it now.",
                crate::platform::secure_store::display_name()
            ));
            self.maybe_link_claude_code_api_key_to_anthropic_provider(api_key)
                .await?;
        } else if has_oauth {
            print_success("✓ Claude Code OAuth session found. Using it now.");
            print_info("  (Token from 'claude login' — typically valid for 8-12 hours)");
        } else {
            print_info("No existing auth found. Claude Code containers need one of these:");
            print_info(&format!(
                "  1. Anthropic API key (stored securely in {})",
                api_key_storage_label
            ));
            print_info("  2. OAuth session from 'claude login' on this machine");
            crate::setup::prompts::print_blank_line();

            if confirm(
                &format!(
                    "Do you want to store an Anthropic API key in {}?",
                    api_key_storage_label
                ),
                true,
            )
            .map_err(SetupError::Io)?
            {
                let api_key = input("Anthropic API key (sk-ant-...)").map_err(SetupError::Io)?;

                if api_key.starts_with("sk-ant-") {
                    #[cfg(target_os = "macos")]
                    {
                        if let Ok(ctx) = self.init_secrets_context().await {
                            match ctx
                                .save_secret(
                                    ANTHROPIC_PROVIDER_SECRET_NAME,
                                    &SecretString::from(api_key.clone()),
                                )
                                .await
                            {
                                Ok(()) => {
                                    print_success(&format!(
                                        "API key encrypted and saved in {}.",
                                        api_key_storage_label
                                    ));
                                    print_info(
                                        "It will be decrypted at runtime using the master key from the OS secure store.",
                                    );
                                    self.enable_anthropic_provider_for_claude_code_key();
                                }
                                Err(e) => {
                                    print_error(&format!(
                                        "Failed to save to {}: {}",
                                        api_key_storage_label, e
                                    ));
                                    print_info(
                                        "Falling back to the OS secure store for compatibility.",
                                    );
                                    match crate::platform::secure_store::store_api_key(
                                        crate::platform::secure_store::CLAUDE_CODE_API_KEY_ACCOUNT,
                                        &api_key,
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            print_success(&format!(
                                                "API key stored securely in the {}.",
                                                crate::platform::secure_store::display_name()
                                            ));
                                            print_info(
                                                "It will be injected into Claude Code containers at runtime.",
                                            );
                                            self.maybe_link_claude_code_api_key_to_anthropic_provider(&api_key)
                                                .await?;
                                        }
                                        Err(store_error) => {
                                            print_error(&format!(
                                                "Failed to store in {}: {}",
                                                crate::platform::secure_store::display_name(),
                                                store_error
                                            ));
                                            print_info(
                                                "You can set ANTHROPIC_API_KEY in your environment instead.",
                                            );
                                        }
                                    }
                                }
                            }
                        } else {
                            print_info(
                                "Encrypted secrets storage is unavailable right now, so ThinClaw will use the OS secure store instead.",
                            );
                            match crate::platform::secure_store::store_api_key(
                                crate::platform::secure_store::CLAUDE_CODE_API_KEY_ACCOUNT,
                                &api_key,
                            )
                            .await
                            {
                                Ok(()) => {
                                    print_success(&format!(
                                        "API key stored securely in the {}.",
                                        crate::platform::secure_store::display_name()
                                    ));
                                    print_info(
                                        "It will be injected into Claude Code containers at runtime.",
                                    );
                                    self.maybe_link_claude_code_api_key_to_anthropic_provider(
                                        &api_key,
                                    )
                                    .await?;
                                }
                                Err(e) => {
                                    print_error(&format!(
                                        "Failed to store in {}: {}",
                                        crate::platform::secure_store::display_name(),
                                        e
                                    ));
                                    print_info(
                                        "You can set ANTHROPIC_API_KEY in your environment instead.",
                                    );
                                }
                            }
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    match crate::platform::secure_store::store_api_key(
                        crate::platform::secure_store::CLAUDE_CODE_API_KEY_ACCOUNT,
                        &api_key,
                    )
                    .await
                    {
                        Ok(()) => {
                            print_success(&format!(
                                "API key stored securely in the {}.",
                                crate::platform::secure_store::display_name()
                            ));
                            print_info(
                                "It will be injected into Claude Code containers at runtime.",
                            );
                            self.maybe_link_claude_code_api_key_to_anthropic_provider(&api_key)
                                .await?;
                        }
                        Err(e) => {
                            print_error(&format!(
                                "Failed to store in {}: {}",
                                crate::platform::secure_store::display_name(),
                                e
                            ));
                            print_info(
                                "You can set ANTHROPIC_API_KEY in your environment instead.",
                            );
                        }
                    }
                } else {
                    print_error(
                        "That doesn't look like an Anthropic API key (expected sk-ant-...).",
                    );
                    print_info("You can set ANTHROPIC_API_KEY in your environment later.");
                }
            } else {
                print_info("No API key stored. You can:");
                print_info("  • Run 'claude login' to set up OAuth");
                print_info("  • Set ANTHROPIC_API_KEY in your environment");
            }
        }

        // ── Model ────────────────────────────────────────────────────────
        crate::setup::prompts::print_blank_line();
        let default_claude_model = crate::config::ClaudeCodeConfig::default().model;
        let model = optional_input(
            "Claude Code model",
            Some(&format!("default: {}", default_claude_model)),
        )
        .map_err(SetupError::Io)?;
        if let Some(m) = model
            && !m.is_empty()
        {
            self.settings.claude_code_model = Some(m);
        }

        // Max turns
        let turns = optional_input("Maximum turns", Some("default: 50")).map_err(SetupError::Io)?;
        if let Some(t) = turns
            && let Ok(n) = t.parse::<u32>()
        {
            self.settings.claude_code_max_turns = Some(n);
        }

        let model_display = self
            .settings
            .claude_code_model
            .as_deref()
            .unwrap_or(default_claude_model.as_str());
        let turns_display = self.settings.claude_code_max_turns.unwrap_or(50);
        print_success(&format!(
            "Claude Code enabled (model: {}, max turns: {})",
            model_display, turns_display
        ));

        Ok(())
    }

    pub(super) async fn step_claude_code(&mut self) -> Result<(), SetupError> {
        self.step_claude_code_inner(true).await
    }

    async fn step_codex_code_inner(&mut self, prompt_enable: bool) -> Result<(), SetupError> {
        if !self.settings.sandbox.enabled {
            print_info("Codex requires the Docker sandbox, which is not enabled yet.");
            print_info("Skipping Codex setup.");
            self.settings.codex_code_enabled = false;
            return Ok(());
        }

        print_info("Codex sandbox lets ThinClaw delegate coding tasks");
        print_info("to OpenAI's Codex CLI running inside a Docker container.");
        print_info(
            "It works best with an OpenAI API key and can also reuse a local Codex auth file.",
        );
        crate::setup::prompts::print_blank_line();

        if prompt_enable && !confirm("Enable Codex sandbox?", false).map_err(SetupError::Io)? {
            self.settings.codex_code_enabled = false;
            print_info("Codex disabled. You can turn it on later with CODEX_CODE_ENABLED=true.");
            return Ok(());
        }

        self.settings.codex_code_enabled = true;

        crate::setup::prompts::print_blank_line();
        print_info("═══ Codex Authentication ═══");
        crate::setup::prompts::print_blank_line();

        let api_key_storage_label = if cfg!(target_os = "macos") {
            format!(
                "ThinClaw's encrypted secrets store (protected by {})",
                crate::platform::secure_store::display_name()
            )
        } else {
            crate::platform::secure_store::display_name().to_string()
        };
        let env_api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let saved_api_key = self
            .resolve_provider_secret_value("OPENAI_API_KEY", OPENAI_PROVIDER_SECRET_NAME)
            .await
            .filter(|value| !value.trim().is_empty());
        let keychain_api_key = if cfg!(target_os = "macos") && saved_api_key.is_some() {
            None
        } else {
            crate::platform::secure_store::get_api_key(
                crate::platform::secure_store::CODEX_CODE_API_KEY_ACCOUNT,
            )
            .await
            .filter(|value| !value.trim().is_empty())
        };
        let auth_path = crate::config::CodexCodeConfig::resolved_auth_file_path();
        let has_auth_file = auth_path.is_file();

        if let Some(api_key) = env_api_key.as_deref() {
            print_success("✓ OPENAI_API_KEY found in the environment. Using it now.");
            self.maybe_link_codex_code_api_key_to_openai_provider(api_key)
                .await?;
        } else if saved_api_key.is_some() {
            print_success(&format!(
                "✓ OpenAI API key found in {}. Using it now.",
                api_key_storage_label
            ));
            self.enable_openai_provider_for_codex_code_key();
        } else if let Some(api_key) = keychain_api_key.as_deref() {
            print_success(&format!(
                "✓ OpenAI API key found in the {}. Using it now.",
                crate::platform::secure_store::display_name()
            ));
            self.maybe_link_codex_code_api_key_to_openai_provider(api_key)
                .await?;
        } else if has_auth_file {
            print_success("✓ Codex auth file found. Using it now.");
            print_info(&format!("  ({})", auth_path.display()));
        } else {
            print_info("No existing auth found. Codex containers need one of these:");
            print_info(&format!(
                "  1. OpenAI API key (stored securely in {})",
                api_key_storage_label
            ));
            print_info(&format!("  2. Codex auth file at {}", auth_path.display()));
            crate::setup::prompts::print_blank_line();

            if confirm(
                &format!(
                    "Do you want to store an OpenAI API key in {}?",
                    api_key_storage_label
                ),
                true,
            )
            .map_err(SetupError::Io)?
            {
                let api_key = input("OpenAI API key (sk-...)").map_err(SetupError::Io)?;
                if api_key.starts_with("sk-") {
                    #[cfg(target_os = "macos")]
                    {
                        if let Ok(ctx) = self.init_secrets_context().await {
                            match ctx
                                .save_secret(
                                    OPENAI_PROVIDER_SECRET_NAME,
                                    &SecretString::from(api_key.clone()),
                                )
                                .await
                            {
                                Ok(()) => {
                                    print_success(&format!(
                                        "API key encrypted and saved in {}.",
                                        api_key_storage_label
                                    ));
                                    print_info(
                                        "It will be decrypted at runtime using the master key from the OS secure store.",
                                    );
                                    self.enable_openai_provider_for_codex_code_key();
                                }
                                Err(e) => {
                                    print_error(&format!(
                                        "Failed to save to {}: {}",
                                        api_key_storage_label, e
                                    ));
                                    print_info(
                                        "Falling back to the OS secure store for compatibility.",
                                    );
                                    match crate::platform::secure_store::store_api_key(
                                        crate::platform::secure_store::CODEX_CODE_API_KEY_ACCOUNT,
                                        &api_key,
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            print_success(&format!(
                                                "API key stored securely in the {}.",
                                                crate::platform::secure_store::display_name()
                                            ));
                                            print_info(
                                                "It will be injected into Codex containers at runtime.",
                                            );
                                            self.maybe_link_codex_code_api_key_to_openai_provider(
                                                &api_key,
                                            )
                                            .await?;
                                        }
                                        Err(store_error) => {
                                            print_error(&format!(
                                                "Failed to store in {}: {}",
                                                crate::platform::secure_store::display_name(),
                                                store_error
                                            ));
                                            print_info(
                                                "You can set OPENAI_API_KEY in your environment instead.",
                                            );
                                        }
                                    }
                                }
                            }
                        } else {
                            print_info(
                                "Encrypted secrets storage is unavailable right now, so ThinClaw will use the OS secure store instead.",
                            );
                            match crate::platform::secure_store::store_api_key(
                                crate::platform::secure_store::CODEX_CODE_API_KEY_ACCOUNT,
                                &api_key,
                            )
                            .await
                            {
                                Ok(()) => {
                                    print_success(&format!(
                                        "API key stored securely in the {}.",
                                        crate::platform::secure_store::display_name()
                                    ));
                                    print_info(
                                        "It will be injected into Codex containers at runtime.",
                                    );
                                    self.maybe_link_codex_code_api_key_to_openai_provider(&api_key)
                                        .await?;
                                }
                                Err(e) => {
                                    print_error(&format!(
                                        "Failed to store in {}: {}",
                                        crate::platform::secure_store::display_name(),
                                        e
                                    ));
                                    print_info(
                                        "You can set OPENAI_API_KEY in your environment instead.",
                                    );
                                }
                            }
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    match crate::platform::secure_store::store_api_key(
                        crate::platform::secure_store::CODEX_CODE_API_KEY_ACCOUNT,
                        &api_key,
                    )
                    .await
                    {
                        Ok(()) => {
                            print_success(&format!(
                                "API key stored securely in the {}.",
                                crate::platform::secure_store::display_name()
                            ));
                            print_info("It will be injected into Codex containers at runtime.");
                            self.maybe_link_codex_code_api_key_to_openai_provider(&api_key)
                                .await?;
                        }
                        Err(e) => {
                            print_error(&format!(
                                "Failed to store in {}: {}",
                                crate::platform::secure_store::display_name(),
                                e
                            ));
                            print_info("You can set OPENAI_API_KEY in your environment instead.");
                        }
                    }
                } else {
                    print_error("That doesn't look like an OpenAI API key (expected sk-...).");
                    print_info("You can set OPENAI_API_KEY in your environment later.");
                }
            } else {
                print_info("No API key stored. You can:");
                print_info("  • Set OPENAI_API_KEY in your environment");
                print_info("  • Run Codex locally so it creates an auth file");
            }
        }

        crate::setup::prompts::print_blank_line();
        let model = optional_input("Codex model", Some("default: gpt-5.3-codex"))
            .map_err(SetupError::Io)?;
        if let Some(m) = model
            && !m.is_empty()
        {
            self.settings.codex_code_model = Some(m);
        }

        let model_display = self
            .settings
            .codex_code_model
            .as_deref()
            .unwrap_or("gpt-5.3-codex");
        print_success(&format!("Codex enabled (model: {})", model_display));

        Ok(())
    }

    pub(super) async fn step_codex_code(&mut self) -> Result<(), SetupError> {
        self.step_codex_code_inner(true).await
    }
}
