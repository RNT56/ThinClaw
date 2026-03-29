# Setup / Onboarding Specification

This document is the authoritative specification for ThinClaw's onboarding
wizard. Any code change to `src/setup/` **must** keep this document in sync.
If a future contributor or coding agent modifies setup behavior, update this
file first, then adjust the code to match.

---

## Entry Points

```
thinclaw onboard [--skip-auth] [--channels-only]
```

Explicit invocation. Loads `.env` files, runs the wizard, exits.

```
thinclaw          (first run, no database configured)
```

Auto-detection via `check_onboard_needed()` in `main.rs`. Skips onboarding
when `ONBOARD_COMPLETED` env var is set (written to `~/.thinclaw/.env` by
the wizard). Otherwise triggers when no database is configured:
- `DATABASE_URL` env var is set
- `LIBSQL_PATH` env var is set
- `~/.thinclaw/thinclaw.db` exists on disk

The `--no-onboard` CLI flag suppresses auto-detection.

---

## Startup Sequence (main.rs)

```
1. Parse CLI args
2. If Command::Onboard  → load .env, run wizard, exit
3. If Command::Run or no command:
   a. Load .env files (dotenvy::dotenv() then load_thinclaw_env())
   b. check_onboard_needed() → run wizard if needed
   c. Config::from_env()     → build config from env vars
   d. ... rest of agent startup
```

**Critical ordering:** `.env` files must be loaded (step 3a) before
`Config::from_env()` (step 3c) because bootstrap vars like
`DATABASE_BACKEND` live in `~/.thinclaw/.env`.

---

## The 18-Step Wizard

### Overview

```
Step 1:  Database Connection
Step 2:  Security (master key)
Step 3:  Agent Identity (name)
Step 4:  Inference Provider          ← skipped if --skip-auth
Step 5:  Model Selection
Step 6:  Embeddings
Step 7:  Channel Configuration
Step 8:  Extensions (tools)
Step 9:  Local Tools & Docker Sandbox
Step 10: Tool Approval Mode
Step 11: Routines (scheduled tasks)
Step 12: Skills
Step 13: Claude Code Sandbox
Step 14: Smart Routing (cheap model)
Step 15: Web UI (theme, accent)
Step 16: Observability
Step 17: Background Tasks (heartbeat)
Step 18: Notification Preferences
        ↓
   save_and_summarize()
```

`--channels-only` mode runs only Step 7, skipping everything else.

---

### Step 1: Database Connection

**Module:** `wizard.rs` → `step_database()`

**Goal:** Select backend, establish connection, run migrations.

**Decision tree:**

```
Both features compiled?
├─ Yes → DATABASE_BACKEND env var set?
│  ├─ Yes → use that backend
│  └─ No  → interactive selection (PostgreSQL vs libSQL)
├─ Only postgres feature → step_database_postgres()
└─ Only libsql feature  → step_database_libsql()
```

**PostgreSQL path** (`step_database_postgres`):
1. Check `DATABASE_URL` from env or settings
2. Test connection (creates `deadpool_postgres::Pool`)
3. Optionally run refinery migrations
4. Store pool in `self.db_pool`

**libSQL path** (`step_database_libsql`):
1. Offer local path (default: `~/.thinclaw/thinclaw.db`)
2. Optional Turso cloud sync (URL + auth token)
3. Test connection (creates `LibSqlBackend`)
4. Always run migrations (idempotent CREATE IF NOT EXISTS)
5. Store backend in `self.db_backend`

**Invariant:** After Step 1, exactly one of `self.db_pool` or
`self.db_backend` is `Some`. This is required for settings persistence
in `save_and_summarize()`.

---

### Step 2: Security (Master Key)

**Module:** `wizard.rs` → `step_security()`

**Goal:** Configure encryption for API tokens and secrets.

**Decision tree:**

```
SECRETS_MASTER_KEY env var set?
├─ Yes → use env var, done
└─ No  → try get_master_key() from OS keychain
   ├─ Ok(bytes) → cache in self.secrets_crypto, ask "use existing?"
   │  ├─ Yes → done (keychain)
   │  └─ No  → clear cache, fall through to options
   └─ Err   → fall through to options
              ├─ OS Keychain: generate + store + build SecretsCrypto
              ├─ Env variable: generate + print export command
              └─ Skip: disable secrets features
```

**CRITICAL CAVEAT: macOS Keychain Dialogs**

On macOS, `security_framework::get_generic_password()` can trigger TWO
system dialogs:
1. "Enter your password to unlock the keychain" (keychain locked)
2. "Allow thinclaw to access this keychain item" (per-app authorization)

This is OS-level behavior we cannot prevent. To minimize pain:

- **Use `get_master_key()` not `has_master_key()`** in step 2. Both call
  the same underlying API, but `get_master_key()` returns the key bytes
  so we can cache them. `has_master_key()` throws them away, forcing a
  second keychain access later.

- **Build `SecretsCrypto` eagerly.** When the keychain key is retrieved,
  immediately construct `SecretsCrypto` and store in `self.secrets_crypto`.
  Later calls to `init_secrets_context()` check this field first, avoiding
  redundant keychain probes.

- **Never probe the keychain in read-only commands** (e.g., `thinclaw status`).
  The status command reports "env not set (keychain may be configured)"
  rather than triggering system dialogs.

**Invariant:** After Step 2, `self.secrets_crypto` is `Some` if the user
chose Keychain or generated a new key. It may be `None` if the user chose
env-var mode or skipped secrets.

---

### Step 4: Inference Provider

**Module:** `wizard.rs` → `step_inference_provider()`

**Goal:** Choose LLM backend and authenticate.

**Providers:**

| Provider | Auth Method | Secret Name | Env Var |
|----------|-------------|-------------|---------|
| Anthropic | API key | `anthropic_api_key` | `ANTHROPIC_API_KEY` |
| OpenAI | API key | `openai_api_key` | `OPENAI_API_KEY` |
| Ollama | None | - | - |
| OpenRouter¹ | API key | `llm_compatible_api_key` | `LLM_API_KEY` |
| OpenAI-compatible¹ | Optional API key | `llm_compatible_api_key` | `LLM_API_KEY` |
| Tinfoil | API key | `tinfoil_api_key` | `TINFOIL_API_KEY` |

¹ OpenRouter and OpenAI-compatible share the same secret name and env var because
OpenRouter is stored as `llm_backend = "openai_compatible"` under the hood.
Switching between them overwrites the same credential slot.

**OpenRouter** (`setup_openrouter`):
- Pre-configured OpenAI-compatible preset with base URL `https://openrouter.ai/api/v1`
- Delegates to `setup_api_key_provider()` with a display name override ("OpenRouter")
- Sets `llm_backend = "openai_compatible"` and `openai_compatible_base_url` automatically
- Clears `selected_model` so Step 4 prompts for a model name (manual text input, no API-based model fetching)

**API-key providers** (`setup_api_key_provider`):
1. Check env var → if set, ask to reuse, persist to secrets store
2. Otherwise prompt for key entry via `secret_input()`
3. Store encrypted in secrets via `init_secrets_context()`
4. **Cache key in `self.llm_api_key`** for model fetching in Step 4

**`self.llm_api_key` caching:** The wizard caches the API key as
`Option<SecretString>` so that Step 4 (model fetching) and Step 5
(embeddings) can use it without re-reading from the secrets store or
mutating environment variables.

---

### Step 5: Model Selection

**Module:** `wizard.rs` → `step_model_selection()`

**Goal:** Choose which model to use.

**Flow:**
1. If model already set → offer to keep it
2. Fetch models from provider API (5-second timeout)
3. On timeout or error → use static fallback list
4. Present list + "Custom model ID" escape hatch
5. Store in `self.settings.selected_model`

**Model fetchers pass the cached API key explicitly:**
```rust
let cached = self.llm_api_key.as_ref().map(|k| k.expose_secret().to_string());
let models = fetch_anthropic_models(cached.as_deref()).await;
```

This avoids mutating environment variables. The fetcher checks the explicit
key first, then falls back to the standard env var.

---

### Step 6: Embeddings

**Module:** `wizard.rs` → `step_embeddings()`

**Goal:** Configure semantic search for workspace memory.

**Flow:**
1. Ask "Enable semantic search?" (default: yes)
2. Detect available providers:
   - OpenAI: if `OPENAI_API_KEY` in env OR (backend is `openai` AND cached key)
3. If available → use it
4. If not → disable embeddings

**Default model:** `text-embedding-3-small` (for both providers)

---

### Step 7: Channel Configuration

**Module:** `wizard.rs` → `step_channels()`, delegating to `channels.rs`

**Goal:** Enable input channels (TUI, HTTP, Telegram, etc.).

**Sub-steps:**

```
6a. Tunnel setup (if webhook channels needed)
6b. Discover WASM channels from ~/.thinclaw/channels/
6c. Build channel options: discovered + bundled + registry catalog
6d. Multi-select: CLI/TUI, HTTP, all available channels
6e. Install missing bundled channels (copy WASM binaries)
6f. Install missing registry channels (download artifacts, fallback to source build)
6g. Initialize SecretsContext (for token storage)
6h. Setup HTTP webhook (if selected)
6i. Setup each WASM channel (secrets, owner binding)
```

**Channel sources** (priority order for installation):
1. Already installed in `~/.thinclaw/channels/`
2. Bundled channels (pre-compiled in `channels-src/`)
3. Registry channels (`registry/channels/*.json`, download-first with source fallback)

**Tunnel setup** (`setup_tunnel`):
- Options: ngrok, Cloudflare Tunnel, localtunnel, custom URL
- Validates HTTPS requirement
- Stored in `self.settings.tunnel.public_url`

**WASM channel setup** (`setup_wasm_channel`):
- Reads `capabilities.json` for `setup.required_secrets`
- For each secret: check existing, prompt or auto-generate, validate regex
- Save each secret via `SecretsContext`

**Telegram special case** (`setup_telegram`):
- Validates bot token via Telegram `getMe` API
- Owner binding: polls `getUpdates` for 120s to capture sender's user ID
- Optional webhook secret generation

**Discord setup** (inline in `step_channels`):
- Requires bot token from Discord Developer Portal
- Checks `DISCORD_BOT_TOKEN` env var for existing token
- Optional: guild ID (restrict to single server)
- Optional: allowed channel IDs (comma-separated)
- Token stored in secrets store + settings

**Slack setup** (inline in `step_channels`):
- Requires both Bot Token (`xoxb-...`) and App-Level Token (`xapp-...`)
- Checks `SLACK_BOT_TOKEN` and `SLACK_APP_TOKEN` env vars
- Optional: allowed channel/DM IDs (comma-separated)
- Both tokens stored in secrets store + settings

**Nostr setup** (inline in `step_channels`):
- Prompts for relay URLs (default: `wss://relay.damus.io,wss://nos.lol`)
- Optional: allowed public keys (hex/npub format)
- Reminds user to set `NOSTR_SECRET_KEY` before starting

**Gmail setup** (inline in `step_channels`):
- Requires GCP project ID, Pub/Sub subscription ID, and topic ID
- Optional: allowed sender email addresses
- Reminds user to run `thinclaw auth gmail` for OAuth2

**iMessage setup** (macOS only, inline in `step_channels`):
- Only shown on macOS builds
- Optional: allowed contacts (phone/email)
- Configurable polling interval (default: 5s)
- Reminds user about Full Disk Access in System Settings

**SecretsContext creation** (`init_secrets_context`):
1. Check `self.secrets_crypto` (set in Step 2) → use if available
2. Else try `SECRETS_MASTER_KEY` env var
3. Else try `get_master_key()` from keychain (only in `channels_only` mode)
4. Create backend-appropriate secrets store (respects selected database backend)

---

### Step 8: Extensions (Tools)

**Module:** `wizard.rs` → `step_extensions()`

**Goal:** Install WASM tools from the extension registry.

**Flow:**
1. Load `RegistryCatalog` from `registry/` directory
2. If registry not found, print info and skip
3. List all tool manifests from the catalog
4. Discover already-installed tools in `~/.thinclaw/tools/`
5. Multi-select: show all registry tools with display name, auth method,
   and description. Pre-check tools tagged `"default"` and already installed.
6. For each selected tool not yet installed, install via
   `RegistryInstaller::install_with_source_fallback()` (download-first,
   fallback to source build)
7. Print consolidated auth hints (deduplicated by provider, e.g. one hint
   for all Google tools sharing `google_oauth_token`)

**Registry lookup** (`load_registry_catalog`):
Searches for `registry/` directory in order:
1. Current working directory
2. Next to the executable
3. `CARGO_MANIFEST_DIR` (compile-time, dev builds)

---

### Step 9: Docker Sandbox

**Module:** `wizard.rs` → `step_sandbox()`

**Goal:** Configure Docker execution sandbox for isolated tool execution.

**Flow:**
1. Ask "Enable Docker sandbox?" (default: no)
2. If yes: configure image, memory limits, timeout, CPU limits
3. Store in `self.settings.sandbox`

---

### Step 10: Routines (Scheduled Tasks)

**Module:** `wizard.rs` → `step_routines()`

**Goal:** Enable or disable the routines system for cron-style scheduled tasks.

**Flow:**
1. Ask "Enable routines?" (default: yes)
2. Store in `self.settings.routines_enabled`

---

### Step 11: Skills

**Module:** `wizard.rs` → `step_skills()`

**Goal:** Enable or disable the skills system (composable capability plugins).

**Flow:**
1. Ask "Enable skills system?" (default: yes)
2. Store in `self.settings.skills_enabled`

---

### Step 12: Claude Code Sandbox

**Module:** `wizard.rs` → `step_claude_code()`

**Goal:** Enable Claude Code delegation sandbox.

**Flow:**
1. Skip if Docker sandbox was disabled in Step 9
2. Ask "Enable Claude Code sandbox?" (default: no)
3. If yes: configure model (default: sonnet) and max turns (default: 50)
4. Store in `self.settings.claude_code_enabled`, `.claude_code_model`, `.claude_code_max_turns`

---

### Step 13: Smart Routing

**Module:** `wizard.rs` → `step_smart_routing()`

**Goal:** Configure a cheap/fast model for lightweight tasks (routing, heartbeat, eval).

**Flow:**
1. Ask "Configure a cheap model for smart routing?" (default: no)
2. If yes: prompt for model in `provider/model` format
3. Store in `self.settings.providers.cheap_model`

---

### Step 14: Web UI

**Module:** `wizard.rs` → `step_web_ui()`

**Goal:** Customize the gateway web dashboard appearance.

**Flow:**
1. Ask "Customize web UI appearance?" (default: no)
2. If yes: select theme (system/light/dark), optional accent color, branding toggle
3. Store in `self.settings.webchat_theme`, `.webchat_accent_color`, `.webchat_show_branding`

---

### Step 15: Observability

**Module:** `wizard.rs` → `step_observability()`

**Goal:** Select the event and metric recording backend.

**Flow:**
1. Select backend: None (default), Log (structured events via tracing)
2. Store in `self.settings.observability_backend`

---

### Step 16: Heartbeat

**Module:** `wizard.rs` → `step_heartbeat()`

**Goal:** Configure periodic background execution.

**Flow:**
1. Ask "Enable heartbeat?" (default: no)
2. If yes: interval in minutes (default: 30), notification channel
3. Store in `self.settings.heartbeat`

---

## Settings Persistence

### Two-Layer Architecture

Settings are persisted in two places:

**Layer 1: `~/.thinclaw/.env`** (bootstrap vars)

Contains only the settings needed BEFORE database connection. Written by
`save_bootstrap_env()` in `bootstrap.rs`.

```env
DATABASE_BACKEND="libsql"
LIBSQL_PATH="/Users/name/.thinclaw/thinclaw.db"
LLM_BACKEND="openai_compatible"
LLM_BASE_URL="http://my-vllm:8000/v1"
```

Or for PostgreSQL:
```env
DATABASE_BACKEND="postgres"
DATABASE_URL="postgres://user:pass@localhost/thinclaw"
LLM_BACKEND="openai_compatible"
```

Or for Ollama:
```env
LLM_BACKEND="ollama"
OLLAMA_BASE_URL="http://localhost:11434"
```

**Why separate?** Chicken-and-egg: you need `DATABASE_BACKEND` to know
which database to connect to, and `LLM_BACKEND` to know how to
configure the provider — neither can be stored in the database.

**Layer 2: Database settings table** (everything else)

All other settings are stored as key-value pairs in the `settings` table,
keyed by `(user_id, key)`. Written by `set_all_settings()`.

Settings are serialized via `Settings::to_db_map()` as dotted paths:
```
database_backend = "libsql"
llm_backend = "openai_compatible"
selected_model = "anthropic/claude-sonnet-4-5"
embeddings.enabled = "true"
embeddings.provider = "openai"
channels.http_enabled = "true"
heartbeat.enabled = "true"
heartbeat.interval_secs = "300"
```

### Incremental Persistence

Settings are persisted **after every successful step**, not just at the end.
This prevents data loss if a later step fails (e.g., the user enters an
API key in step 3 but step 5 crashes — they won't need to re-enter it).

**`persist_after_step()`** is called after each step in `run()` and:
1. Writes bootstrap vars to `~/.thinclaw/.env` via `write_bootstrap_env()`
2. Writes all current settings to the database via `persist_settings()`
3. Silently ignores errors (e.g., if called before Step 1 establishes a DB)

**`try_load_existing_settings()`** is called after Step 1 establishes a
database connection. It loads any previously saved settings from the
database using `get_all_settings("default")` → `Settings::from_db_map()`
→ `merge_from()`. This recovers progress from prior partial wizard runs.

**Ordering after Step 1 is critical:**

```
step_database()                        → sets DB fields in self.settings
let step1 = self.settings.clone()      → snapshot Step 1 choices
try_load_existing_settings()           → merge DB values into self.settings
self.settings.merge_from(&step1)       → re-apply Step 1 (fresh wins over stale)
persist_after_step()                   → save merged state
```

This ordering ensures:
- Prior progress (steps 2-7 from a previous partial run) is recovered
- Fresh Step 1 choices override stale DB values (not the reverse)
- The first DB persist doesn't clobber prior settings with defaults

### save_and_summarize()

Final step of the wizard:

```
1. Mark onboard_completed = true
2. Call persist_settings() for final write (idempotent — ensures
   onboard_completed flag is saved)
3. Call write_bootstrap_env() for final .env write (idempotent)
4. Print configuration summary
```

Bootstrap vars written to `~/.thinclaw/.env`:
- `DATABASE_BACKEND` (always)
- `DATABASE_URL` (if postgres)
- `LIBSQL_PATH` (if libsql)
- `LIBSQL_URL` (if turso sync)
- `LLM_BACKEND` (always, when set)
- `LLM_BASE_URL` (if openai_compatible)
- `OLLAMA_BASE_URL` (if ollama)
- `ONBOARD_COMPLETED` (always, "true")

**Invariant:** Both Layer 1 and Layer 2 must be written. If the database
write fails, the wizard returns an error and the `.env` file is not written.

### Legacy Migration

`bootstrap.rs` handles one-time upgrades from older config formats:
- `bootstrap.json` → extracts `DATABASE_URL`, writes `.env`, renames to `.migrated`
- `settings.json` → migrated to database via `migrate_disk_to_db()`

---

## Settings Struct

**Module:** `settings.rs`

```rust
pub struct Settings {
    // Meta
    pub onboard_completed: bool,

    // Step 1: Database
    pub database_backend: Option<String>,    // "postgres" | "libsql"
    pub database_url: Option<String>,
    pub libsql_path: Option<String>,
    pub libsql_url: Option<String>,

    // Step 2: Security
    pub secrets_master_key_source: KeySource, // Keychain | Env | None

    // Step 3: Inference
    pub llm_backend: Option<String>,         // "anthropic" | "openai" | "ollama" | "openai_compatible" | "tinfoil"
    pub ollama_base_url: Option<String>,
    pub openai_compatible_base_url: Option<String>,

    // Step 4: Model
    pub selected_model: Option<String>,

    // Step 5: Embeddings
    pub embeddings: EmbeddingsSettings,      // enabled, provider, model

    // Step 6: Channels
    pub tunnel: TunnelSettings,              // provider, public_url
    pub channels: ChannelSettings,           // http config, telegram owner, etc.

    // Step 7: Heartbeat
    pub heartbeat: HeartbeatSettings,        // enabled, interval, notify

    // Advanced (not in wizard, set via `thinclaw config set`)
    pub agent: AgentSettings,
    pub wasm: WasmSettings,
    pub sandbox: SandboxSettings,
    pub safety: SafetySettings,
    pub builder: BuilderSettings,
}
```

**KeySource enum:** `Keychain | Env | None`

---

## Secrets Flow

### SecretsContext

Thin wrapper for setup-time secret operations:

```rust
pub struct SecretsContext {
    store: Arc<dyn SecretsStore>,
    user_id: String,
}
```

Created by `init_secrets_context()` which:
1. Gets `SecretsCrypto` from `self.secrets_crypto` or loads from keychain/env
2. Creates the appropriate backend store:
   - If both features compiled: respects `self.settings.database_backend`
   - Tries selected backend first, falls back to the other
3. Returns `SecretsContext` wrapping the store

### Secret Storage

Secrets are encrypted with AES-256-GCM using the master key, then stored
in the database `secrets` table. The wizard writes secrets like:

```
telegram_bot_token    → encrypted bot token
telegram_webhook_secret → encrypted webhook HMAC secret
anthropic_api_key     → encrypted API key
```

---

## Prompt Utilities

**Module:** `prompts.rs`

| Function | Description |
|----------|-------------|
| `select_one(label, options)` | Numbered single-choice menu |
| `select_many(label, options, defaults)` | Checkbox multi-select (raw terminal mode) |
| `input(label)` | Single line text input |
| `optional_input(label, hint)` | Text input that can be empty |
| `secret_input(label)` | Hidden input (shows `*` per char), returns `SecretString` |
| `confirm(label, default)` | `[Y/n]` or `[y/N]` prompt |
| `print_header(text)` | Bold section header with underline |
| `print_step(n, total, text)` | `[1/7] Step Name` |
| `print_success(text)` | Green `✓` prefix (ANSI color), message in default color |
| `print_error(text)` | Red `✗` prefix (ANSI color), message in default color |
| `print_info(text)` | Blue `ℹ` prefix (ANSI color), message in default color |

`select_many` uses `crossterm` raw mode for arrow key navigation.
Must properly restore terminal state on all exit paths.

---

## Platform Caveats

### macOS Keychain

- `get_generic_password()` triggers system dialogs (unlock + authorize)
- Two dialogs per call is normal, not a bug
- Cache the result after first access to avoid repeat prompts
- Never probe keychain in read-only commands (`status`, `--help`)
- Service name: `"thinclaw"`, account: `"master_key"`

### Linux Secret Service

- Uses GNOME Keyring or KWallet via `secret-service` crate
- May need `gnome-keyring` daemon running
- Collection unlock may prompt for password

### Remote Server Authentication

On remote/VPS servers without a desktop environment, use providers that
authenticate via API keys (Anthropic, OpenAI, OpenRouter, Tinfoil) rather
than browser-based OAuth flows.

For custom OAuth callbacks, set `THINCLAW_OAUTH_CALLBACK_URL` to a
publicly accessible URL:
```bash
export THINCLAW_OAUTH_CALLBACK_URL=https://myserver.example.com:9876
```

The `callback_url()` function in `oauth_defaults.rs` checks this env var
and falls back to `http://127.0.0.1:{OAUTH_CALLBACK_PORT}`.

### URL Passwords

- `#` is common in URL-encoded passwords (`%23` decoded)
- `.env` values must be double-quoted to preserve `#`
- Display masked: `postgres://user:****@host/db`

### Telegram API

- Bot token format: `123456:ABC-DEF...`
- Token goes in URL path: `https://api.telegram.org/bot{TOKEN}/method`
- Webhook secret header: `X-Telegram-Bot-Api-Secret-Token`
- Owner binding polls `getUpdates` (must delete webhook first)

---

## Testing

Tests live in `mod tests {}` at the bottom of each file.

**What to test when modifying setup:**

- Settings round-trip: `to_db_map()` then `from_db_map()` preserves values
- Bootstrap `.env`: dotenvy can parse what `save_bootstrap_env()` writes
- Model fetchers: static fallback works when API is unreachable
- Channel discovery: handles missing dir, invalid JSON, deduplication
- Prompt functions: not tested (interactive I/O), but ensure error paths
  don't panic

**Run setup tests:**
```bash
cargo test --lib -- setup
cargo test --lib -- bootstrap
```

---

## Modification Checklist

When changing the onboarding flow:

1. Update this README first with the intended behavior change
2. If adding a new wizard step:
   - Add to the step enum in `run()`, adjust `total_steps`
   - Add corresponding settings fields to `Settings`
   - Add `to_db_map` / `from_db_map` serialization
   - If the setting is needed before DB connection, add to `save_bootstrap_env()`
3. If adding a new provider or channel:
   - Add to the selection menu in the appropriate step
   - Add authentication flow (API key or OAuth)
   - Add model fetcher with static fallback + 5s timeout
4. If touching keychain:
   - Cache the result, never call `get_master_key()` twice
   - Test on macOS (dialog behavior differs from Linux)
5. If touching secrets:
   - Ensure `init_secrets_context()` respects the selected database backend
   - Test with both postgres and libsql features
6. Run the full shipping checklist:
   ```bash
   cargo fmt
   cargo clippy --all --benches --tests --examples --all-features -- -D warnings
   cargo test --lib -- setup bootstrap
   ```
7. Test a fresh onboarding: `rm -rf ~/.thinclaw && cargo run`
