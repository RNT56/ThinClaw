# Setup Wizard: First-Run Onboarding

The setup wizard guides users through initial configuration on first launch. ThinClaw provides **two variants** of the same wizard: a **Tauri UI flow** (graphical, multi-step form) and a **terminal wizard** (for headless/SSH setups using `inquire` prompts). Both call the same underlying configuration functions.

---

## 1. What OpenClaw's Wizard Does Today

OpenClaw's `src/wizard/` (13 files, ~77KB) provides a comprehensive CLI onboarding flow:

### Wizard Steps (from `onboarding.ts`)

1. **Risk Acknowledgement** — Security warning about tool access, data exposure, and multi-user risks. User must explicitly accept.
2. **Flow Selection** — QuickStart (accept sensible defaults) or Advanced (configure everything manually).
3. **Existing Config Handling** — If config exists: keep, update, or reset (with scope: config-only, config+creds, or full).
4. **Workspace Directory** — Where agent data, sessions, and memory live (default: `~/.openclaw`).
5. **Auth / Provider Setup** — Choose auth method: API key, token provider, or skip. Enter API keys for cloud providers.
6. **Model Selection** — Pick default model from discovered models (with provider preference hinting).
7. **Gateway Configuration** (from `onboarding.gateway-config.ts`):
   - Port selection (default: 8080)
   - Bind mode: loopback, LAN, Tailnet, auto, custom IP
   - Auth mode: token (recommended) or password
   - Tailscale exposure: off, serve, or funnel
   - Safety constraints (funnel→password, tailscale→loopback)
   - Dangerous node command denylist (camera, screen record, contacts)
8. **Channel Setup** — Configure messaging channels (Telegram token, Discord bot, etc.)
9. **Skills Setup** — Install default skills
10. **Internal Hooks** — Enable session memory on `/new`
11. **Finalization** (from `onboarding.finalize.ts`):
    - Write `config.toml`
    - Create workspace directory
    - Generate BOOTSTRAP.md template
    - Install shell completions
    - Install as OS daemon (optional)
    - Launch the TUI or print next-steps

---

## 2. ThinClaw Wizard: Tauri UI Variant

The graphical wizard is a multi-step form rendered in the Tauri WebView.

### Screen Flow

```
┌─────────────────────────────────────────┐
│  1. Welcome                             │
│  ─────────────────────────────────      │
│  Welcome to ThinClaw!                   │
│                                         │
│  ThinClaw is a personal AI agent that   │
│  runs on your machine. It can browse    │
│  the web, execute code, manage files,   │
│  and connect to your chat platforms.    │
│                                         │
│  ⚠️ Security Notice:                    │
│  ThinClaw can access your files and     │
│  run commands when tools are enabled.   │
│  Review security docs before enabling   │
│  tool access for untrusted users.       │
│                                         │
│  [  I understand, continue  →  ]        │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  2. Setup Mode                          │
│  ─────────────────────────────────      │
│                                         │
│  ◉ QuickStart (recommended)             │
│    Sensible defaults, configure later   │
│                                         │
│  ○ Advanced                             │
│    Configure everything step by step    │
│                                         │
│  ○ Remote Orchestrator                  │
│    Connect to a remote ThinClaw server  │
│                                         │
│  [  ← Back  ]         [  Next →  ]      │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  3. AI Provider                         │
│  ─────────────────────────────────      │
│                                         │
│  How would you like to power your AI?   │
│                                         │
│  ◉ Cloud Provider (OpenAI, Anthropic)   │
│    ┌──────────────────────────────┐     │
│    │ API Key: ●●●●●●●●●●●●●●●●   │     │
│    │ Provider: [OpenAI ▼]         │     │
│    └──────────────────────────────┘     │
│                                         │
│  ○ Local Inference Only (MLX)           │
│    No API keys needed, runs on-device   │
│                                         │
│  ○ Both (Cloud + Local fallback)        │
│                                         │
│  [  ← Back  ]         [  Next →  ]      │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  4. Model Selection                     │
│  ─────────────────────────────────      │
│                                         │
│  Select your default model:             │
│                                         │
│  ┌──────────────────────────────────┐   │
│  │ ◉ claude-3-5-sonnet   (Anthropic) │  │
│  │ ○ gpt-4o              (OpenAI)    │  │
│  │ ○ gpt-4o-mini         (OpenAI)    │  │
│  │ ○ deepseek-v3    (OpenRouter)     │  │
│  │ ○ Llama-3-8B-MLX      (Local)     │  │
│  └──────────────────────────────────┘   │
│                                         │
│  💡 You can change this anytime from    │
│     Settings or with /model command.    │
│                                         │
│  [  ← Back  ]         [  Next →  ]      │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  5. Agent Identity                      │
│  ─────────────────────────────────      │
│                                         │
│  Name: [ ThinClaw             ]         │
│                                         │
│  Personality (SOUL.md):                 │
│  ┌──────────────────────────────────┐   │
│  │ You are a helpful personal AI    │   │
│  │ assistant. You are friendly,     │   │
│  │ concise, and proactive.          │   │
│  └──────────────────────────────────┘   │
│  📝 Edit the full SOUL.md later in      │
│     ~/.thinclaw/workspace/SOUL.md       │
│                                         │
│  [  ← Back  ]         [  Next →  ]      │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  6. Channels (Optional)                 │
│  ─────────────────────────────────      │
│                                         │
│  Connect messaging platforms:           │
│                                         │
│  ☐ Telegram   [  Configure  ]           │
│  ☐ Discord    [  Configure  ]           │
│  ☐ Signal     [  Configure  ]           │
│  ☐ Slack      [  Configure  ]           │
│  ☐ Nostr      [  Configure  ]           │
│  ☐ iMessage   [  Configure  ]           │
│                                         │
│  💡 You can add channels later from     │
│     Settings or with /channels add.     │
│                                         │
│  [  ← Back  ]    [  Skip  ] [  Next →]  │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  7. Networking (Advanced only)          │
│  ─────────────────────────────────      │
│                                         │
│  Port:  [ 8080     ]                    │
│  Bind:  [ Loopback (127.0.0.1)  ▼ ]    │
│  Auth:  [ Token (recommended)   ▼ ]    │
│                                         │
│  Token: [auto-generated] [Copy] [Regen] │
│                                         │
│  Tailscale:                             │
│  ○ Off                                  │
│  ○ Serve (expose to Tailnet)            │
│  ○ Funnel (expose to internet)          │
│                                         │
│  [  ← Back  ]         [  Next →  ]      │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  8. Review & Launch                     │
│  ─────────────────────────────────      │
│                                         │
│  ✅ Provider: OpenAI (cloud)            │
│  ✅ Model: gpt-4o                       │
│  ✅ Agent: ThinClaw                     │
│  ✅ Port: 8080 (loopback)              │
│  ✅ Auth: Token                         │
│  ✅ Channels: Telegram                  │
│  ✅ Workspace: ~/.thinclaw              │
│                                         │
│  ☐ Install as system service            │
│    (auto-start on boot)                 │
│                                         │
│  [  ← Back  ]    [  🚀 Launch  ]        │
└─────────────────────────────────────────┘
```

### Tauri IPC Commands

```rust
#[tauri::command]
async fn wizard_validate_api_key(
    provider: String,
    key: String,
) -> Result<ValidateKeyResult, String> {
    // Test the key by making a lightweight API call
    let client = create_provider_client(&provider, &key)?;
    match client.list_models().await {
        Ok(models) => Ok(ValidateKeyResult {
            valid: true,
            models: models.into_iter().map(|m| m.id).collect(),
            error: None,
        }),
        Err(e) => Ok(ValidateKeyResult {
            valid: false,
            models: vec![],
            error: Some(e.to_string()),
        }),
    }
}

#[tauri::command]
async fn wizard_discover_local_models() -> Result<Vec<LocalModel>, String> {
    // Scan for MLX/GGUF models
    discover_local_models().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn wizard_finalize(
    state: State<'_, AppState>,
    config: WizardConfig,
) -> Result<(), String> {
    // 1. Write config.toml
    let toml_config = config.to_toml()?;
    write_config_file(&toml_config).await?;

    // 2. Store API keys in Keychain
    if let Some(key) = &config.api_key {
        state.secret_store.set(&config.provider, key).await?;
    }

    // 3. Create workspace directory
    create_workspace(&config.workspace_dir).await?;

    // 4. Write default SOUL.md
    write_soul_template(&config.workspace_dir, &config.agent_name).await?;

    // 5. Optionally install daemon
    if config.install_daemon {
        install_daemon_service().await?;
    }

    // 6. Start Orchestrator
    state.start_orchestrator(toml_config).await?;

    Ok(())
}
```

---

## 3. Terminal Wizard Variant

For headless setups (`thinclaw onboard`), the same flow runs in the terminal using `inquire`:

```rust
use inquire::{Select, Text, Confirm, Password, MultiSelect};

pub async fn run_terminal_wizard(opts: OnboardArgs) -> Result<()> {
    // Step 1: Security acknowledgement
    println!("{}", SECURITY_WARNING);
    let accepted = Confirm::new("I understand the risks. Continue?")
        .with_default(false)
        .prompt()?;
    if !accepted {
        println!("Setup cancelled.");
        return Ok(());
    }

    // Step 2: Flow selection
    let flow = if opts.quickstart {
        WizardFlow::QuickStart
    } else {
        Select::new("Setup mode", vec![
            WizardFlow::QuickStart,
            WizardFlow::Advanced,
            WizardFlow::Remote,
        ])
        .with_help_message("QuickStart accepts sensible defaults")
        .prompt()?
    };

    // Step 3: Provider setup
    let provider_choice = Select::new("How do you want to power your AI?", vec![
        "Cloud Provider (OpenAI, Anthropic, OpenRouter)",
        "Local Inference Only (MLX)",
        "Both (Cloud + Local fallback)",
    ]).prompt()?;

    let api_key = if provider_choice.starts_with("Cloud") || provider_choice.starts_with("Both") {
        let provider = Select::new("Provider", vec!["OpenAI", "Anthropic", "OpenRouter"])
            .prompt()?;
        let key = Password::new(&format!("{} API Key", provider))
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .prompt()?;

        // Validate key
        print!("Validating key... ");
        match validate_api_key(&provider, &key).await {
            Ok(_) => println!("✅ Valid!"),
            Err(e) => {
                println!("❌ Invalid: {}", e);
                return Err(e);
            }
        }
        Some((provider, key))
    } else {
        None
    };

    // Step 4: Model selection
    let models = discover_models(&api_key).await?;
    let model = Select::new("Default model", models)
        .with_help_message("You can change this anytime with /model")
        .prompt()?;

    // Step 5: Gateway config (Advanced only)
    let gateway = if flow == WizardFlow::Advanced {
        let port: u16 = Text::new("Gateway port")
            .with_default("8080")
            .prompt()?
            .parse()?;

        let bind = Select::new("Gateway bind", vec![
            "Loopback (127.0.0.1)",
            "LAN (0.0.0.0)",
            "Tailnet (Tailscale IP)",
            "Custom IP",
        ]).prompt()?;

        let auth = Select::new("Auth mode", vec![
            "Token (recommended)",
            "Password",
        ]).prompt()?;

        let tailscale = Select::new("Tailscale exposure", vec![
            "Off", "Serve", "Funnel",
        ]).prompt()?;

        GatewayConfig { port, bind: parse_bind(bind), auth: parse_auth(auth)?, tailscale: parse_ts(tailscale) }
    } else {
        GatewayConfig::default()
    };

    // Step 6: Channels (optional)
    let channels = MultiSelect::new("Connect channels (optional)", vec![
        "Telegram", "Discord", "Signal", "Slack", "Nostr", "iMessage",
    ])
    .with_help_message("Press space to select, enter to confirm. Skip with enter.")
    .prompt()?;

    // Configure each selected channel
    for channel in &channels {
        configure_channel(channel).await?;
    }

    // Step 7: Finalize
    let install_daemon = Confirm::new("Install as system service (auto-start on boot)?")
        .with_default(false)
        .prompt()?;

    finalize_wizard(WizardConfig {
        provider: api_key.as_ref().map(|(p, _)| p.clone()),
        api_key: api_key.as_ref().map(|(_, k)| k.clone()),
        model,
        gateway,
        channels,
        install_daemon,
        ..Default::default()
    }).await?;

    println!("\n🚀 ThinClaw is ready! Run `thinclaw tui` to start chatting.");
    Ok(())
}
```

---

## 4. Shared Finalization Logic

Both wizard variants call the same `finalize_wizard` function:

```rust
pub async fn finalize_wizard(config: WizardConfig) -> Result<()> {
    let workspace_dir = config.workspace_dir.clone()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".thinclaw"));

    // 1. Create directories
    tokio::fs::create_dir_all(&workspace_dir).await?;
    tokio::fs::create_dir_all(workspace_dir.join("sessions")).await?;
    tokio::fs::create_dir_all(workspace_dir.join("skills")).await?;
    tokio::fs::create_dir_all(workspace_dir.join("memory")).await?;

    // 2. Write config.toml
    let config_path = dirs::config_dir().unwrap().join("thinclaw/config.toml");
    tokio::fs::create_dir_all(config_path.parent().unwrap()).await?;
    let toml = config.to_toml()?;
    tokio::fs::write(&config_path, toml).await?;

    // 3. Store secrets in Keychain
    if let Some(key) = &config.api_key {
        let store = SecretStore::new()?;
        store.set(&format!("{}_api_key", config.provider.as_deref().unwrap_or("default")), key)?;
    }

    // Generate gateway token
    let token = generate_random_token();
    let store = SecretStore::new()?;
    store.set("gateway_token", &token)?;

    // 4. Write SOUL.md template
    let soul_path = workspace_dir.join("SOUL.md");
    if !soul_path.exists() {
        let template = include_str!("../../docs/reference/templates/SOUL.md");
        tokio::fs::write(&soul_path, template).await?;
    }

    // 5. Write BOOTSTRAP.md template
    let bootstrap_path = workspace_dir.join("BOOTSTRAP.md");
    if !bootstrap_path.exists() {
        let template = include_str!("../../docs/reference/templates/BOOTSTRAP.md");
        tokio::fs::write(&bootstrap_path, template).await?;
    }

    // 6. Install shell completions (if terminal wizard)
    #[cfg(feature = "cli")]
    install_shell_completions()?;

    // 7. Install as daemon
    if config.install_daemon {
        #[cfg(target_os = "macos")]
        install_launchd_service(&std::env::current_exe()?, &config_path)?;

        #[cfg(target_os = "linux")]
        install_systemd_service(&std::env::current_exe()?, &config_path)?;
    }

    // 8. Run initial security audit
    let auditor = SecurityAuditor::new(config.to_app_config()?, workspace_dir.clone());
    let report = auditor.run().await;
    if report.summary.critical > 0 {
        tracing::warn!(
            "Security audit found {} critical issues. Run `thinclaw security audit` for details.",
            report.summary.critical
        );
    }

    Ok(())
}
```

---

## 5. Re-Onboarding / Reconfiguration

The wizard supports re-running on an existing installation:

```rust
pub enum ExistingConfigAction {
    /// Keep current config, skip to channels
    Keep,
    /// Update specific values (merge with existing)
    Update,
    /// Reset everything
    Reset(ResetScope),
}

pub enum ResetScope {
    ConfigOnly,              // Delete config.toml only
    ConfigAndCredentials,    // Delete config + Keychain entries
    Full,                    // Delete config + creds + workspace + sessions
}
```

Both the Tauri Settings page and `thinclaw onboard --reset` trigger this flow.

---

## 6. Dangerous Command Denylist

During setup, new installations automatically block high-risk node commands:

```rust
const DEFAULT_DENIED_COMMANDS: &[&str] = &[
    "camera.snap",
    "camera.clip",
    "screen.record",
    "calendar.add",
    "contacts.add",
    "reminders.add",
];
```

Users can arm these via `/phone arm camera.snap` or in the Settings UI.

---

## 7. Crate Dependencies

```toml
[dependencies]
inquire = "0.7"           # Terminal prompts (for headless wizard)
rand = "0.8"              # Token generation
```
