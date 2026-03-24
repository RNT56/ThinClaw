> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# Rebuilding OpenClaw Configuration in Rust

It is a common misconception that OpenClaw's behavior is coded into the macOS or iOS native applications. **It is not.**

The companion applications (macOS, iOS, Android) are fully "dumb" terminals. They contain _zero_ logic about how the AI behaves, which API keys to use, or what chat channels exist. The only configuration the macOS app stores is:

1. The IP address of the Gateway.
2. An auto-generated TLS certificate to securely connect to the Gateway.
3. User permissions (e.g., "Did the user grant camera access?").

Everything else is centralized in the Node.js sidecar.

## The OpenClaw Configuration Architecture

In OpenClaw, there is a single source of truth for the entire system: **`openclaw.json5`** (usually located at `~/.openclaw/openclaw.json5`).

When the Node.js agent boots up, it reads this file and parses over 30 distinct types of configuration (as seen in `src/config/types.ts`):

1. **`agents`**: Defines the personalities, system prompts, and default tools for the main agent and sub-agents.
2. **`auth`**: Holds the API keys for OpenAI, Anthropic, local MLX, etc.
3. **`channels`**: Contains the bot tokens and channel IDs for Telegram, Discord, Slack, Signal, and Nostr.
4. **`tools`** & **`skills`**: Booleans and settings indicating which native tools (like `browser-tool`) or external skills the agent is allowed to use.
5. **`gateway`**: Network bindings, TLS configurations, and mDNS exact settings for broadcasting to the macOS app.

## How to Transfer this to Rust

In Node.js, OpenClaw uses `zod` to validate the massive `json5` file at runtime. In Rust, this is significantly easier and more performant because of **`serde`**.

### 1. Shift from JSON5 to TOML

JSON5 is great for humans, but `TOML` is the de facto standard for Rust configuration files. It supports comments, is highly readable, and perfectly maps to Rust's nested struct system.

**Target File: `~/.thinclaw/config.toml`**

### 2. The Rust Struct Hierarchy

You will use the `serde` crate to strictly deserialize the TOML file into application state.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ThinClawConfig {
    pub gateway: GatewayConfig,
    pub agent: AgentConfig,
    pub auth: AuthConfig,
    pub channels: ChannelConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub port: u16,
    pub mDNS_domain: String,
    pub tls_cert_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentConfig {
    pub default_model: String,
    pub system_prompt: String,
    pub tool_timeout_seconds: u32,
    pub max_context_tokens: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChannelConfig {
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
    pub signal: Option<SignalConfig>,
}

/// ⚠️ IMPORTANT: `bot_token` is NEVER stored in config.toml.
/// Only non-sensitive metadata lives here. Secrets are stored in the
/// macOS/Linux Keychain via the `keyring` crate (see SECRETS_RS.md).
/// The `secret_ref` field holds the Keychain key *name* (not the key itself),
/// which the Orchestrator resolves at runtime:
///   SecretStore::get_key(&config.telegram.secret_ref)  →  "telegram_bot_token"
#[derive(Debug, Deserialize, Serialize)]
pub struct TelegramConfig {
    /// Keychain reference name. The actual token is fetched from SecretStore.
    /// Example value: "telegram_bot_token"
    pub secret_ref: String,
    pub allowed_user_ids: Vec<String>,
}
```

### 3. Loading the Config in Rust

Loading and validating the config requires almost zero manual logic. `figment` or `config-rs` can load the file, apply environment variable overrides, and hydrate your structs instantly.

```rust
// Cargo.toml
// serde = { version = "1.0", features = ["derive"] }
// figment = { version = "0.10", features = ["toml", "env"] }

use figment::{Figment, providers::{Format, Toml, Env}};

// Load from ~/.thinclaw/config.toml, allowing environment variables (e.g. THINCLAW_TELEGRAM_BOT_TOKEN) to override file settings.
let config: ThinClawConfig = Figment::new()
    .merge(Toml::file(dirs::home_dir().unwrap().join(".thinclaw/config.toml")))
    .merge(Env::prefixed("THINCLAW_"))
    .extract()?;
```

## Migration Strategy

You do not need to port all of `openclaw.json5` at once.

Because you are starting with your own RIG agent, simply create a minimal `ThinClawConfig` struct in Rust with only the things you need _right now_ (e.g., API keys, System Prompts, and the Telegram bot token). As you port more channels (Slack, Discord) to Rust, you just add `Option<SlackConfig>` to your `ThinClawConfig` struct.
