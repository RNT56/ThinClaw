//! Setup Wizard: first-run onboarding for IronClaw.
//!
//! Guides the user through initial configuration via terminal prompts
//! using `inquire`. Writes settings to `~/.ironclaw/config.toml` and
//! stores API keys via the secrets store.
//!
//! Both a QuickStart flow (sensible defaults) and an Advanced flow
//! (step-by-step configuration) are supported.

use std::fmt;
use std::path::PathBuf;

use inquire::{Confirm, Password, Select, Text};

use crate::settings::Settings;

// ── Types ────────────────────────────────────────────────────────────

/// Setup mode selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupMode {
    QuickStart,
    Advanced,
}

impl fmt::Display for SetupMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupMode::QuickStart => write!(f, "QuickStart — sensible defaults, configure later"),
            SetupMode::Advanced => write!(f, "Advanced — configure everything step by step"),
        }
    }
}

/// AI provider selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderChoice {
    OpenAI,
    Anthropic,
    OpenRouter,
    Ollama,
    Skip,
}

impl fmt::Display for ProviderChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderChoice::OpenAI => write!(f, "OpenAI (GPT-4o, GPT-4o-mini)"),
            ProviderChoice::Anthropic => write!(f, "Anthropic (Claude 3.5 Sonnet)"),
            ProviderChoice::OpenRouter => write!(f, "OpenRouter (multiple providers)"),
            ProviderChoice::Ollama => write!(f, "Ollama (local models, no API key)"),
            ProviderChoice::Skip => write!(f, "Skip — I'll configure this later"),
        }
    }
}

/// Result of the wizard.
#[derive(Debug, Clone)]
pub struct WizardResult {
    /// Settings to save.
    pub settings: Settings,
    /// API key to store (provider, key).
    pub api_key: Option<(String, String)>,
    /// Workspace directory.
    pub workspace_dir: PathBuf,
}

// ── Security Warning ─────────────────────────────────────────────────

const SECURITY_WARNING: &str = "\
╔══════════════════════════════════════════════════════════════╗
║  ⚠  IronClaw Security Notice                                ║
╠══════════════════════════════════════════════════════════════╣
║                                                              ║
║  IronClaw is a personal AI agent that can:                   ║
║                                                              ║
║  • Execute shell commands on your machine                    ║
║  • Read and write files on your filesystem                   ║
║  • Browse the web and make HTTP requests                     ║
║  • Access connected messaging platforms                      ║
║                                                              ║
║  These capabilities are controlled by the safety system      ║
║  and tool approval settings. Review your configuration       ║
║  carefully, especially when enabling tools for untrusted     ║
║  users or exposing the agent to the network.                 ║
║                                                              ║
╚══════════════════════════════════════════════════════════════╝";

// ── Wizard Implementation ────────────────────────────────────────────

/// Run the terminal setup wizard.
///
/// Returns `Ok(Some(result))` if the wizard completed successfully,
/// `Ok(None)` if the user cancelled, or `Err` on inquire errors.
pub fn run_wizard(quickstart: bool) -> Result<Option<WizardResult>, String> {
    println!("\n🔧 IronClaw Setup Wizard\n");

    // Step 1: Security acknowledgement
    println!("{SECURITY_WARNING}\n");
    let accepted = Confirm::new("I understand the above. Continue with setup?")
        .with_default(false)
        .prompt()
        .map_err(|e| format!("Prompt error: {e}"))?;

    if !accepted {
        println!("Setup cancelled.");
        return Ok(None);
    }

    // Step 2: Flow selection
    let mode = if quickstart {
        SetupMode::QuickStart
    } else {
        Select::new(
            "Setup mode:",
            vec![SetupMode::QuickStart, SetupMode::Advanced],
        )
        .with_help_message(
            "QuickStart accepts sensible defaults; Advanced lets you configure everything",
        )
        .prompt()
        .map_err(|e| format!("Prompt error: {e}"))?
    };

    let mut settings = Settings::default();

    // Step 3: AI Provider
    let provider = Select::new(
        "How do you want to power your AI?",
        vec![
            ProviderChoice::Anthropic,
            ProviderChoice::OpenAI,
            ProviderChoice::OpenRouter,
            ProviderChoice::Ollama,
            ProviderChoice::Skip,
        ],
    )
    .with_help_message("You can change this later with /model or in config.toml")
    .prompt()
    .map_err(|e| format!("Prompt error: {e}"))?;

    let api_key = match &provider {
        ProviderChoice::OpenAI => {
            settings.llm_backend = Some("openai".to_string());
            prompt_api_key("OpenAI")?
        }
        ProviderChoice::Anthropic => {
            settings.llm_backend = Some("anthropic".to_string());
            prompt_api_key("Anthropic")?
        }
        ProviderChoice::OpenRouter => {
            settings.llm_backend = Some("openai_compatible".to_string());
            settings.openai_compatible_base_url = Some("https://openrouter.ai/api/v1".to_string());
            prompt_api_key("OpenRouter")?
        }
        ProviderChoice::Ollama => {
            settings.llm_backend = Some("ollama".to_string());
            let base_url = Text::new("Ollama base URL:")
                .with_default("http://localhost:11434")
                .prompt()
                .map_err(|e| format!("Prompt error: {e}"))?;
            settings.ollama_base_url = Some(base_url);
            None
        }
        ProviderChoice::Skip => None,
    };

    // Step 4: Model selection
    let model = match &provider {
        ProviderChoice::Anthropic => Select::new(
            "Default model:",
            vec![
                "claude-sonnet-4-20250514",
                "claude-3-5-sonnet-20241022",
                "claude-3-5-haiku-20241022",
            ],
        )
        .prompt()
        .map_err(|e| format!("Prompt error: {e}"))?
        .to_string(),
        ProviderChoice::OpenAI => Select::new(
            "Default model:",
            vec!["gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "o3-mini"],
        )
        .prompt()
        .map_err(|e| format!("Prompt error: {e}"))?
        .to_string(),
        ProviderChoice::OpenRouter => {
            Text::new("Default model (e.g. 'anthropic/claude-3.5-sonnet'):")
                .with_default("anthropic/claude-3.5-sonnet")
                .prompt()
                .map_err(|e| format!("Prompt error: {e}"))?
        }
        ProviderChoice::Ollama => Text::new("Default model (e.g. 'llama3.1'):")
            .with_default("llama3.1")
            .prompt()
            .map_err(|e| format!("Prompt error: {e}"))?,
        ProviderChoice::Skip => "default".to_string(),
    };
    settings.selected_model = Some(model);

    // Step 5: Agent name (advanced only)
    if mode == SetupMode::Advanced {
        let name = Text::new("Agent name:")
            .with_default("ironclaw")
            .with_help_message("Your agent's display name")
            .prompt()
            .map_err(|e| format!("Prompt error: {e}"))?;
        settings.agent.name = name;
    }

    // Step 6: Workspace directory
    let default_workspace = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw");

    let workspace_dir = if mode == SetupMode::Advanced {
        let path = Text::new("Workspace directory:")
            .with_default(&default_workspace.to_string_lossy())
            .with_help_message("Where IronClaw stores data, sessions, and memory")
            .prompt()
            .map_err(|e| format!("Prompt error: {e}"))?;
        PathBuf::from(path)
    } else {
        default_workspace
    };

    // Mark onboarding as complete
    settings.onboard_completed = true;

    // Step 7: Review
    println!("\n━━━ Configuration Summary ━━━\n");
    println!(
        "  Provider:  {}",
        settings.llm_backend.as_deref().unwrap_or("not configured")
    );
    println!(
        "  Model:     {}",
        settings.selected_model.as_deref().unwrap_or("default")
    );
    println!("  Agent:     {}", settings.agent.name);
    println!("  Workspace: {}", workspace_dir.display());
    if api_key.is_some() {
        println!("  API Key:   ●●●●●●●● (will be stored securely)");
    }
    println!();

    let confirm = Confirm::new("Save this configuration?")
        .with_default(true)
        .prompt()
        .map_err(|e| format!("Prompt error: {e}"))?;

    if !confirm {
        println!("Setup cancelled.");
        return Ok(None);
    }

    Ok(Some(WizardResult {
        settings,
        api_key,
        workspace_dir,
    }))
}

/// Prompt for an API key.
fn prompt_api_key(provider: &str) -> Result<Option<(String, String)>, String> {
    let key = Password::new(&format!("{provider} API Key:"))
        .with_display_mode(inquire::PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()
        .map_err(|e| format!("Prompt error: {e}"))?;

    if key.is_empty() {
        return Ok(None);
    }

    Ok(Some((provider.to_lowercase(), key)))
}

/// Finalize the wizard: create directories and write config.
pub async fn finalize_wizard(result: &WizardResult) -> Result<(), String> {
    // Create workspace directories
    let dirs_to_create = [
        result.workspace_dir.clone(),
        result.workspace_dir.join("sessions"),
        result.workspace_dir.join("skills"),
        result.workspace_dir.join("memory"),
    ];

    for dir in &dirs_to_create {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    }

    // Write config.toml
    let config_path = Settings::default_toml_path();
    result
        .settings
        .save_toml(&config_path)
        .map_err(|e| format!("Failed to write config: {e}"))?;

    println!("  ✅ Configuration saved to {}", config_path.display());
    println!(
        "  ✅ Workspace created at {}",
        result.workspace_dir.display()
    );

    // Store API key if provided
    if let Some((provider, key)) = &result.api_key {
        // Write to .env file for now (secrets store integration deferred)
        let env_path = result.workspace_dir.join(".env");
        let env_var = match provider.as_str() {
            "openai" => "OPENAI_API_KEY",
            "anthropic" => "ANTHROPIC_API_KEY",
            "openrouter" => "OPENROUTER_API_KEY",
            other => {
                // Generic fallback
                let var = format!("{}_API_KEY", other.to_uppercase());
                let content = format!("{var}={key}\n");
                tokio::fs::write(&env_path, content)
                    .await
                    .map_err(|e| format!("Failed to write .env: {e}"))?;
                println!("  ✅ API key saved to {}", env_path.display());
                return Ok(());
            }
        };
        let content = format!("{env_var}={key}\n");

        // Append to existing .env or create new
        let existing = tokio::fs::read_to_string(&env_path)
            .await
            .unwrap_or_default();
        let new_content = if existing.contains(env_var) {
            // Replace existing line
            existing
                .lines()
                .map(|line| {
                    if line.starts_with(env_var) {
                        format!("{env_var}={key}")
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        } else {
            format!("{existing}{content}")
        };

        tokio::fs::write(&env_path, new_content)
            .await
            .map_err(|e| format!("Failed to write .env: {e}"))?;

        // Set restrictive permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o600));
        }

        println!("  ✅ API key saved to {}", env_path.display());
    }

    // Write default SOUL.md if it doesn't exist
    let soul_path = result.workspace_dir.join("SOUL.md");
    if !soul_path.exists() {
        let template = format!(
            "# {name}\n\n\
             You are {name}, a helpful personal AI assistant.\n\n\
             ## Personality\n\
             - Friendly and approachable\n\
             - Concise but thorough\n\
             - Proactive — suggest next steps when appropriate\n\n\
             ## Guidelines\n\
             - Always confirm before destructive operations\n\
             - Explain your reasoning when making decisions\n\
             - Ask for clarification when requirements are ambiguous\n",
            name = result.settings.agent.name
        );
        tokio::fs::write(&soul_path, template)
            .await
            .map_err(|e| format!("Failed to write SOUL.md: {e}"))?;
        println!("  ✅ Default SOUL.md created");
    }

    println!("\n🚀 IronClaw is ready! Run `ironclaw` to start chatting.\n");

    Ok(())
}

/// Check if onboarding has been completed.
pub fn is_onboarded() -> bool {
    let settings = Settings::load();
    settings.onboard_completed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_display() {
        let p = ProviderChoice::OpenAI;
        let s = format!("{p}");
        assert!(s.contains("OpenAI"));
    }

    #[test]
    fn test_setup_mode_display() {
        let m = SetupMode::QuickStart;
        assert!(format!("{m}").contains("QuickStart"));
    }

    #[test]
    fn test_security_warning() {
        assert!(SECURITY_WARNING.contains("Security"));
        assert!(SECURITY_WARNING.contains("shell commands"));
    }

    #[test]
    fn test_is_onboarded_default() {
        // Default settings have onboard_completed = false
        let settings = Settings::default();
        assert!(!settings.onboard_completed);
    }
}
