# ThinClaw Agent Flow

> **Last updated:** 2026-03-29
> **Source of truth:** `src/main.rs`, `src/app.rs`, `src/bootstrap.rs`, `src/wizard/mod.rs`,
> `src/setup/wizard.rs`, `src/workspace/mod.rs`, `src/agent/agent_loop.rs`, `src/agent/thread_ops.rs`

---

## Table of Contents

1. [Overview](#1-overview)
2. [Filesystem Layout](#2-filesystem-layout)
3. [Configuration Priority Chain](#3-configuration-priority-chain)
4. [First-Run Bootstrap (Setup Wizard)](#4-first-run-bootstrap-setup-wizard)
5. [Boot Sequence (5-Phase AppBuilder)](#5-boot-sequence-5-phase-appbuilder)
6. [Workspace Seeding (Identity Files)](#6-workspace-seeding-identity-files)
7. [System Prompt Assembly](#7-system-prompt-assembly)
8. [Channel Wiring & Agent Construction](#8-channel-wiring--agent-construction)
9. [Agent Main Loop](#9-agent-main-loop)
10. [Message Processing Pipeline](#10-message-processing-pipeline)
11. [Background Tasks](#11-background-tasks)
12. [Scrappy (Tauri) Embedding](#12-scrappy-tauri-embedding)
13. [Appendix: Complete First-Run Timeline](#13-appendix-complete-first-run-timeline)
14. [Agent Autonomy — Internal Reasoning & Progress Updates](#14-agent-autonomy--internal-reasoning--progress-updates)

---

## 1. Overview

ThinClaw is a personal AI agent that runs as a standalone binary or embedded inside
a macOS Tauri app (Scrappy). The boot process has three major layers:

1. **Infrastructure setup** — database, encryption, LLM connection, tool registry
2. **Identity seeding** — workspace files that define who the agent is
3. **Runtime loop** — channels, message processing, background tasks

```
┌─────────────────────────────────────────────────────────────┐
│                     ThinClaw Boot                           │
│                                                             │
│  ┌──────────┐  ┌──────────────┐  ┌───────────────────────┐  │
│  │ Bootstrap │→│  AppBuilder  │→│  Agent::run()          │  │
│  │ (.env,    │  │  (5 phases)  │  │  (channels, loop,    │  │
│  │  wizard)  │  │              │  │   background tasks)  │  │
│  └──────────┘  └──────────────┘  └───────────────────────┘  │
│                       │                                     │
│                       ▼                                     │
│              workspace.seed_if_empty()                      │
│              (IDENTITY.md, SOUL.md, AGENTS.md, ...)         │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Filesystem Layout

```
~/.thinclaw/                          ← User-level ThinClaw directory
├── .env                              ← Bootstrap env vars (DATABASE_URL, etc.)
│                                       Priority: env vars > ./.env > ~/.thinclaw/.env
├── config.toml                       ← Optional TOML config overlay
├── thinclaw.db                       ← libSQL database (default backend)
│                                       Contains: settings, sessions, workspace docs, secrets
├── skills/                           ← User-level skills (SkillTrust::Trusted)
│   └── my-skill/SKILL.md
├── tools/                            ← WASM tool binaries (.wasm files)
│   └── dev/                          ← Dev tools from build artifacts
├── channels/                         ← WASM channel binaries
│   ├── telegram.wasm
│   ├── slack.wasm
│   ├── whatsapp.wasm
│   └── *.capabilities.json
├── projects/                         ← Docker sandbox project bind-mounts
├── logs/                             ← Service logs (daemon.stdout/stderr.log)
├── tts/                              ← Generated TTS audio output
├── audio/                            ← Voice/audio capture temp files
├── memory_hygiene_state.json         ← Cadence tracker for auto-cleanup
├── telegram-*.json                   ← Telegram pairing and allowlist state
├── settings.json.migrated            ← Legacy config (renamed after migration)
├── bootstrap.json.migrated           ← Legacy bootstrap (renamed after migration)
└── mcp-servers.json.migrated         ← Legacy MCP config (renamed after migration)
```

**Source:** `src/bootstrap.rs` — `thinclaw_env_path()` returns `~/.thinclaw/.env`

---

## 3. Configuration Priority Chain

Settings are resolved in order (highest priority wins):

| Priority | Source | When Loaded | Example |
|----------|--------|-------------|---------|
| 1 (highest) | **Explicit env vars** | Always | `export LLM_BACKEND=openai` |
| 2 | **`./.env` (CWD)** | `dotenvy::dotenv()` | Project-level overrides |
| 3 | **`~/.thinclaw/.env`** | `bootstrap::load_thinclaw_env()` | `DATABASE_URL` |
| 4 | **`config.toml`** | `Config::apply_toml_overlay()` | Structured config |
| 5 | **Injected secrets** | Phase 2 (`inject_llm_keys_from_secrets`) | API keys from Keychain |
| 6 | **Database settings** | Phase 1 (`Config::from_db()`) | Wizard-set values |
| 7 (lowest) | **Compiled defaults** | Always | Sensible defaults |

`dotenvy` never overwrites existing env vars, giving the chain its natural priority.

**Source:** `src/bootstrap.rs:19-41`, `src/config/mod.rs`

---

## 4. First-Run Bootstrap (Setup Wizard)

### 4.1 First-Run Detection

When ThinClaw starts, `check_onboard_needed()` runs two checks:

```rust
// src/main.rs:1233+
fn check_onboard_needed() -> Option<&'static str> {
    let has_db = env::var("DATABASE_URL").is_ok()
        || env::var("LIBSQL_PATH").is_ok()
        || config::default_libsql_path().exists();

    if !has_db { return Some("Database not configured"); }
    if env::var("ONBOARD_COMPLETED") == Ok("true") { return None; }
    Some("First run")
}
```

If onboarding is needed, the **9-step Setup Wizard** launches interactively.

### 4.2 Setup Wizard Steps

The wizard is in `src/setup/wizard.rs`. Settings are **persisted incrementally** after
each step, so if step 6 fails, steps 1–5 are saved and won't be re-asked on retry.

| Step | Name | What It Does |
|------|------|-------------|
| 1 | **Database** | Choose PostgreSQL or libSQL, enter connection URL, test connection, run migrations |
| 2 | **Security** | Generate encryption master key, store in OS Keychain or env var |
| 3 | **Inference Provider** | Pick Anthropic/OpenAI/Ollama/OpenRouter, enter API key |
| 4 | **Model Selection** | Choose default model (e.g. `claude-sonnet-4-20250514`, `gpt-4o`) |
| 5 | **Embeddings** | Enable semantic search (OpenAI or Ollama embeddings) |
| 6 | **Channels** | Configure Telegram, Signal, HTTP gateway, Discord, iMessage, Gmail |
| 7 | **Extensions** | Install WASM tools from the extension registry |
| 8 | **Docker Sandbox** | Enable sandboxed code execution containers |
| 9 | **Background Tasks** | Configure heartbeat interval and notifications |

After the final step, the wizard writes `ONBOARD_COMPLETED=true` to `~/.thinclaw/.env`
so it won't run again.

### 4.3 QuickStart vs Advanced

The wizard offers two modes:

- **QuickStart** — accepts sensible defaults (libSQL local DB, keychain key, picks provider + model)
- **Advanced** — step-by-step with full control over every setting

### 4.4 Legacy Config Migration

On first run, if legacy files exist they are automatically migrated:

| Legacy File | Migration Target | What Happens |
|------------|-----------------|-------------|
| `bootstrap.json` | `~/.thinclaw/.env` | Extracts `DATABASE_URL`, writes `.env`, renames to `.migrated` |
| `settings.json` | DB `settings` table | Calls `Settings::to_db_map()`, stores in DB, renames to `.migrated` |
| `mcp-servers.json` | DB `mcp_servers` key | Stores raw JSON in DB settings, renames to `.migrated` |
| `session.json` | DB `nearai.session_token` | Stores in DB settings, renames to `.migrated` |

**Source:** `src/bootstrap.rs:184+` — `migrate_disk_to_db()`

---

## 5. Boot Sequence (5-Phase AppBuilder)

After the wizard completes (or is skipped), `AppBuilder::build_all()` runs.
This is the core initialization engine.

```
src/main.rs:258-265
  let components = AppBuilder::new(config, flags, toml_path, log_broadcaster)
      .build_all()
      .await?;
```

### Phase 0: Early Bootstrap (before AppBuilder)

```rust
// src/main.rs (early bootstrap, before AppBuilder)
let _ = dotenvy::dotenv();                              // Load ./.env
thinclaw::bootstrap::load_thinclaw_env();               // Load ~/.thinclaw/.env
let config = Config::from_env_with_toml(toml_path)?;    // Env + optional TOML
let log_broadcaster = Arc::new(LogBroadcaster::new());  // For WebLogLayer
init_tracing(Arc::clone(&log_broadcaster));             // Structured logging
```

### Phase 1: `init_database()`

- Creates DB connection pool (PostgreSQL `deadpool` or libSQL in-process)
- Runs schema migrations (idempotent, safe to re-run)
- Migrates legacy `settings.json` → DB rows (one-time, `src/bootstrap.rs:184`)
- **Reloads config from DB** — database settings now layer into the config
- Applies TOML overlay again (env > TOML > DB > defaults)
- Cleans up stale sandbox jobs (if sandbox is enabled)

### Phase 2: `init_secrets()`

Two paths depending on context:

| Mode | How Secrets Work |
|------|-----------------|
| **Scrappy (Tauri)** | SecretsStore is pre-injected by Scrappy from macOS Keychain |
| **Standalone** | Loads master key from Keychain/env, creates `SecretsCrypto` + DB backing |

After secrets are available:
- **Injects API keys** from encrypted storage into config overlay
- **Re-resolves config** — API keys are now available for LLM instantiation

### Phase 3: `init_llm()`

Builds the LLM provider chain with decorator layers:

```
Base Provider (OpenAI / Anthropic / Ollama / OpenAI-compatible / Gemini / llama.cpp)
  └── Retry (exponential backoff)
      └── Smart Routing (latency-based model selection)
          └── Failover (primary → fallback provider)
              └── Circuit Breaker (error rate detection)
                  └── Response Cache (dedup identical requests)
```

Optionally creates a **cheap LLM** (e.g. `gpt-4o-mini`) for lightweight tasks
like heartbeat checks, routing decisions, and evaluation.

Also builds the **Provider Vault** — a runtime-configurable set of LLM providers
managed via the WebUI, with encrypted API key storage and hot-swap.

### Phase 4: `init_tools()`

- Creates **SafetyLayer** (content filtering, PII detection, policy checks)
- Creates **ToolRegistry** with credential injection support
- Registers **builtin tools**: file, search, web, calculator, canvas, browser, agent_think, emit_user_message
- Creates **embedding provider** (OpenAI, Ollama, or Gemini)
- Creates **Workspace** (DB-backed, with embeddings if available)
- Registers **memory tools**: `memory_write`, `memory_read`, `memory_search`, `memory_list`, `memory_delete`
- Registers **builder tool** (if sandbox is enabled)
- Registers **subagent tools**: `spawn_subagent`, `list_subagents`, `cancel_subagent`
- Creates **MediaPipeline** (image, audio, video, PDF extraction and routing)

### Phase 5: `init_extensions()`

- Creates **WASM tool runtime** and loads `.wasm` tools from `~/.thinclaw/tools/`
- Loads dev tools from `tools/dev/` build artifacts
- Connects to configured **MCP servers** (concurrent startup, with auth token injection)
- Loads **extension catalog** (registry entries for in-chat discovery)
- Creates **ExtensionManager** (enables search/install/activate within conversations)
- Registers **TTS tool** (OpenAI text-to-speech)
- Sets up **Claude Code** delegation (Docker sandbox with per-job auth tokens)

### Post-Build: Assembly

After the 5 phases, `build_all()` performs:

```rust
// src/app.rs (post-build assembly)
// 1. Seed workspace identity files
workspace.seed_if_empty().await;    // Creates 7 core files if missing

// 2. Backfill embeddings (background)
tokio::spawn(workspace.backfill_embeddings());  // Generates missing vectors
```

**Source:** `src/app.rs:847+` — `build_all()`

---

## 6. Workspace Seeding (Identity Files)

### 6.1 What Gets Created

`seed_if_empty()` creates **7 core workspace files** in the database on every boot.
It **never overwrites** existing files — only creates missing ones.

| File | Purpose | Loaded into System Prompt? |
|------|---------|---------------------------|
| `README.md` | Workspace structure documentation | No |
| `MEMORY.md` | Long-term curated notes, decisions, facts | Yes (except group chats) |
| `IDENTITY.md` | Agent name, vibe, emoji, personality | Yes |
| `SOUL.md` | Core values, behavioral principles, boundaries | Yes |
| `AGENTS.md` | Session startup instructions (the "bootstrap") | Yes |
| `USER.md` | User's name, timezone, preferences | Yes |
| `HEARTBEAT.md` | Periodic background task checklist | No (read by heartbeat runner) |

### 6.2 Default Content

Each file is seeded with meaningful starter content, not blank templates.

#### `IDENTITY.md`
```markdown
# Identity

- **Name:** (pick one during your first conversation)
- **Vibe:** (how you come across, e.g. calm, witty, direct)
- **Emoji:** (your signature emoji, optional)

Edit this file to give the agent a custom name and personality.
The agent will evolve this over time as it develops a voice.
```

#### `SOUL.md`
```markdown
# Core Values

Be genuinely helpful, not performatively helpful. Skip filler phrases.
Have opinions. Disagree when it matters.
Be resourceful before asking: read the file, check context, search, then ask.
Earn trust through competence. Be careful with external actions, bold with internal ones.
You have access to someone's life. Treat it with respect.

## Boundaries

- Private things stay private. Never leak user context into group chats.
- When in doubt about an external action, ask before acting.
- Prefer reversible actions over destructive ones.
- You are not the user's voice in group settings.
```

#### `AGENTS.md` (the "bootstrap")
```markdown
# Agent Instructions

You are a personal AI assistant with access to tools and persistent memory.

## Every Session

1. Read SOUL.md (who you are)
2. Read USER.md (who you're helping)
3. Read today's daily log for recent context

## Memory

You wake up fresh each session. Workspace files are your continuity.
- Daily logs (`daily/YYYY-MM-DD.md`): raw session notes
- `MEMORY.md`: curated long-term knowledge
Write things down. Mental notes do not survive restarts.

## Guidelines

- Always search memory before answering questions about prior conversations
- Write important facts and decisions to memory for future reference
- Use the daily log for session-level notes
- Be concise but thorough

## Safety

- Do not exfiltrate private data
- Prefer reversible actions over destructive ones
- When in doubt, ask
```

### 6.3 Identity Document Protection

Files marked as **identity documents** get special handling:

```rust
// src/workspace/document.rs:101+
pub fn is_identity_document(&self) -> bool {
    matches!(self.path.as_str(),
        paths::IDENTITY | paths::SOUL | paths::AGENTS | paths::USER
    )
}
```

The **hygiene system** (`src/workspace/hygiene.rs`) automatically cleans up old daily
logs but **never touches identity documents**. This ensures that the agent's personality
and user preferences survive cleanup passes.

### 6.4 How Users Edit Identity Files

The user can edit workspace files through:
- **Chat**: "Update my SOUL.md to add a preference for..." → agent uses `memory_write` tool
- **CLI**: `thinclaw memory write SOUL.md "new content"`
- **Web UI**: Memory browser in the Gateway web interface
- **Scrappy UI**: Settings → Personality editor calling `memory_read`/`memory_write`

**Source:** `src/workspace/mod.rs:947+` — `seed_if_empty()`

---

## 7. System Prompt Assembly

When the agent processes a message, `system_prompt_for_context()` assembles the system
prompt from workspace files:

```rust
// src/workspace/mod.rs:621+
pub async fn system_prompt_for_context(&self, is_group_chat: bool) -> Result<String> {
    let mut parts = Vec::new();

    // 1. Load identity files in order
    let identity_files = [
        (paths::AGENTS,   "## Agent Instructions"),
        (paths::SOUL,     "## Core Values"),
        (paths::USER,     "## User Context"),
        (paths::IDENTITY, "## Identity"),
    ];

    for (path, header) in identity_files {
        if let Ok(doc) = self.read(path).await && !doc.content.is_empty() {
            parts.push(format!("{}\n\n{}", header, doc.content));
        }
    }

    // 2. Load MEMORY.md (excluded in group chats to prevent data leaking)
    if !is_group_chat {
        if let Ok(doc) = self.read(paths::MEMORY).await && !doc.content.is_empty() {
            parts.push(format!("## Long-Term Memory\n\n{}", doc.content));
        }
    }

    // 3. Load last 2 days of daily logs
    for date in [today, yesterday] {
        if let Ok(doc) = self.daily_log(date).await && !doc.content.is_empty() {
            let header = if date == today { "## Today's Notes" } else { "## Yesterday's Notes" };
            parts.push(format!("{}\n\n{}", header, doc.content));
        }
    }

    // 4. Inject active channel names
    parts.push(format!("## Active Channels\n\n{}", active_channels.join(", ")));

    Ok(parts.join("\n\n---\n\n"))
}
```

### Assembled System Prompt Structure

```
## Agent Instructions         ← from AGENTS.md
(session startup instructions, memory usage guidelines)

---

## Core Values                ← from SOUL.md
(behavioral principles, boundaries)

---

## User Context               ← from USER.md
(user's name, timezone, preferences)

---

## Identity                   ← from IDENTITY.md
(agent name, personality, vibe)

---

## Long-Term Memory           ← from MEMORY.md (excluded in group chats)
(curated facts, decisions, preferences)

---

## Today's Notes              ← from daily/YYYY-MM-DD.md
(session notes from today)

---

## Yesterday's Notes          ← from daily/YYYY-MM-DD.md
(session notes from yesterday)

---

## Active Channels            ← injected at runtime
(list of currently active channel names)
```

### Group Chat Privacy

When `is_group_chat` is `true`, `MEMORY.md` is **excluded** from the system prompt.
This prevents private context (user preferences, personal notes) from leaking into
group conversations.

---

## 8. Channel Wiring & Agent Construction

After `build_all()` returns, `main.rs` wires up channels and builds the agent:

```rust
// src/main.rs (channel wiring, simplified)

// 1. Create channel manager
let channels = ChannelManager::new();

// 2. Add channels based on config
if config.channels.cli.enabled { channels.add(ReplChannel::new()); }
if config.channels.signal.is_some() { channels.add(SignalChannel::new()); }
if config.channels.discord.is_some() { channels.add(DiscordChannel::new()); }
if config.channels.imessage.is_some() { channels.add(IMessageChannel::new()); }
if config.channels.gmail.is_some() { channels.add(GmailChannel::new()); }
if config.channels.apple_mail.is_some() { channels.add(AppleMailChannel::new()); }
if config.channels.wasm_channels_enabled { setup_wasm_channels(); }
// WASM channels: Telegram, Slack, WhatsApp (auto-discovered from ~/.thinclaw/channels/)

// 3. Skills discovery
let registry = SkillRegistry::new(~/.thinclaw/skills/)
    .with_installed_dir(installed_dir);
registry.discover_all().await;  // Discover: Workspace > User > Installed

// 4. Hardware bridge (Scrappy injects camera/mic/screen access)
if let Some(bridge) = tool_bridge {
    tools.register(create_bridged_tools(bridge));
}

// 5. Agent registry (multi-agent routing)
let agent_registry = AgentRegistry::new(workspace.clone());
let agent_router = AgentRouter::new(agent_registry);

// 6. Construct the agent
let agent = Agent::new(
    agent_config,
    AgentDeps { store, llm, cheap_llm, safety, tools, workspace, media_pipeline, ... },
    channels,
    heartbeat_config,
    hygiene_config,
    routine_config,
    context_manager,
    session_manager,
    agent_router,
);

// 7. Start web gateway + webhook server
// 8. Enter main loop
agent.run().await
```

---

## 9. Agent Main Loop

`Agent::run()` is the core event loop:

```rust
// src/agent/agent_loop.rs:669+  (simplified)
pub async fn run(self) -> Result<(), Error> {
    // 1. Start all channels (each returns a stream of IncomingMessage)
    let mut message_stream = self.channels.start_all().await?;

    // 2. Start background tasks (self-repair, session pruning, heartbeat,
    //    hygiene, cron routines, backfill embeddings, health monitor)
    let bg = self.start_background_tasks().await;

    // 3. Start config file watcher
    let config_watcher = ConfigWatcher::new(&toml_path);
    config_watcher.start().await;

    // 4. Fire BeforeAgentStart hook
    self.hooks().run(&HookEvent::AgentStart { ... }).await?;

    // 5. Execute BOOT.md hook (runs on every startup)
    self.run_boot_hook().await;

    // 6. Execute BOOTSTRAP.md hook (first run only, deletes file after)
    self.run_bootstrap_hook().await;

    // 7. Main message loop
    loop {
        let message = tokio::select! {
            _ = ctrl_c() => break,                    // Graceful shutdown
            msg = message_stream.next() => match msg {
                Some(m) => m,
                None => break,                        // All channels closed
            }
        };

        match self.handle_message(&message).await {
            Ok(Some(response)) => {
                // Hook: BeforeOutbound — modify/suppress response
                self.channels.respond(&message, response).await;
            }
            Ok(None) => break,    // /quit, /exit, /shutdown
            Err(e) => {
                self.channels.respond(&message, format!("Error: {}", e)).await;
            }
        }

        // Check event triggers (cheap regex match, fires async routines if matched)
    }

    // 8. Shutdown: cancel background tasks, close channels
}
```

---

## 10. Message Processing Pipeline

When a message arrives, `handle_message()` runs this pipeline:

```
IncomingMessage arrives from any channel
│
├── 1. Parse submission type (SubmissionParser::parse)
│       → UserInput, SystemCommand(/status, /job, /model, /restart), Undo, Redo,
│         Interrupt, Compact, Clear, NewThread, Heartbeat,
│         Summarize, Suggest, Quit, SwitchThread, Resume,
│         ExecApproval, ApprovalResponse
│
├── 2. Hook: BeforeInbound (modify/reject input)
│
├── 3. Hydrate thread from DB (if historical thread ID)
│
├── 4. Resolve session + thread (SessionManager)
│
├── 5. Multi-agent routing (AgentRouter)
│       → Determines which agent workspace handles this message
│       → Claims thread ownership (first-responder wins)
│
├── 6. Auth mode interception
│       → If thread is awaiting an OAuth token, route directly
│         to credential store (bypass normal processing)
│
├── 7. Route by submission type:
│   │
│   ├── UserInput → process_user_input()
│   │   ├── Extract media attachments (MediaPipeline)
│   │   │   ├── Images → base64 encode → LLM vision input
│   │   │   ├── Audio → Whisper transcription → text
│   │   │   ├── Video → ffmpeg keyframe extraction → vision input
│   │   │   ├── PDFs/Docs → text extraction → context
│   │   │   └── Stickers → WebP/TGS conversion → image
│   │   ├── Check thread state (Processing → reject, Idle → proceed)
│   │   ├── Safety validation (input content checks)
│   │   ├── Safety policy checks (block/allow rules)
│   │   ├── Command routing (/status, /job → direct handler)
│   │   ├── Auto-compact if context > threshold
│   │   ├── Create undo checkpoint
│   │   ├── Start turn (add user message to thread)
│   │   ├── Persist user message to DB
│   │   ├── Send "Thinking..." status
│   │   ├── Run agentic tool loop (LLM ↔ tools)
│   │   ├── Hook: TransformResponse (modify/reject output)
│   │   ├── Complete turn, persist assistant response
│   │   └── Return response
│   │
│   ├── SystemCommand → handle_system_command()
│   ├── Undo/Redo → restore from checkpoint
│   ├── Interrupt → cancel processing
│   ├── Compact → manual context compaction
│   ├── Clear → wipe thread history
│   ├── Heartbeat → run heartbeat check
│   └── Quit → shutdown signal
│
├── 8. Convert SubmissionResult to response string
│       → Suppress silent replies (group chat "nothing to say")
│       → Format approval prompts (tools needing user confirmation)
│
├── 9. Channel-aware formatting
│       → Telegram: Markdown → HTML (markdown_to_telegram_html)
│       → Slack: Markdown → mrkdwn (markdown_to_slack_mrkdwn)
│       → WhatsApp: Markdown → WhatsApp text (markdown_to_whatsapp)
│       → Discord: pass-through (native Markdown support)
│       → Others: plain text
│
└── 10. Hook: BeforeOutbound (modify/suppress response)
         → Send response back through the originating channel
```

### Agentic Tool Loop

The inner `run_agentic_loop()` executes the LLM ↔ tool cycle:

```
Build messages (system prompt + thread history + user input)
│
└── Loop:
    ├── Call LLM with messages + available tool definitions
    ├── If response contains tool_use:
    │   ├── Safety check (is tool auto-approved? needs user approval?)
    │   ├── If needs approval → pause loop, return NeedApproval
    │   ├── Execute tool → get result
    │   ├── Safety: sanitize tool output
    │   ├── Send ToolStarted/ToolCompleted/ToolResult status events
    │   ├── Append tool result to messages
    │   └── Continue loop (LLM sees tool result, may call more tools)
    │
    └── If response is text-only → return response, exit loop
```

---

## 11. Background Tasks

`start_background_tasks()` spawns several concurrent tasks:

| Task | Interval | What It Does |
|------|----------|-------------|
| **Self-repair** | `repair_check_interval` (default: 60s) | Detects stuck jobs and broken tools, attempts auto-recovery |
| **Session pruning** | Every 10 minutes | Removes idle sessions past `session_idle_timeout` |
| **Heartbeat** | `heartbeat.interval_secs` (default: 3600s) | Reads HEARTBEAT.md, runs tasks via cheap LLM, sends notifications |
| **Memory hygiene** | `hygiene.cadence_hours` (default: 12h) | Deletes daily logs older than `retention_days` (default: 30) |
| **Cron routines** | Configurable cron expressions | Runs scheduled routines (e.g. daily summary, inbox check) |
| **Embedding backfill** | Once at startup | Generates embeddings for unembedded document chunks |
| **Config watcher** | Filesystem events | Watches `config.toml` for changes, logs reload notification |
| **Channel health monitor** | Periodic | Checks channel health, auto-restarts with failure tracking + cooldown |
| **Zombie reaper** | Every 60s | Aborts stuck routine tasks exceeding max duration |
| **Docker orphan cleanup** | Once at startup | Kills orphaned `thinclaw-worker` containers from previous crashes |

**Source:** `src/agent/agent_loop.rs` (background task setup, before `run()`)

---

## 12. Scrappy (Tauri) Embedding

When Scrappy embeds ThinClaw, the flow differs slightly:

### What Scrappy Provides

| Component | How It's Injected |
|-----------|------------------|
| **SecretsStore** | Pre-created from macOS Keychain, passed via `with_secrets_store()` |
| **ToolBridge** | Scrappy's sensor access (camera, mic, screen), passed via `with_tool_bridge()` |
| **LogBroadcaster** | Shared for Tauri log window |

### What Scrappy Skips

- **Setup Wizard** — Scrappy has its own onboarding UI
- **CLI channels** — no REPL, no terminal; all interaction via Tauri IPC
- **Web gateway** — no HTTP server; Scrappy uses `invoke()` / `listen()` / `emit()`

### What's Automatic

- **`seed_if_empty()`** — workspace identity files are created automatically inside `build_all()`
- **System prompt assembly** — works identically to standalone mode
- **Background tasks** — heartbeat, hygiene, cron all run normally

### Boot Timeline (Scrappy)

```
Scrappy::main()
├── Create config from settings + ThinClaw env
├── AppBuilder::new(config, flags, None, log_broadcaster)
│   .with_secrets_store(keychain_store)      // macOS Keychain
│   .with_tool_bridge(sensor_bridge)         // Camera/mic/screen
│   .build_all().await
│   │
│   ├── Phase 1-5 (same as standalone)
│   └── workspace.seed_if_empty()            // Identity files created
│
├── Agent::new(config, deps, channels, ...)
├── tokio::spawn(agent.run())                // Background
│
└── Scrappy UI ready
    ├── User messages → invoke("openclaw_send_message")
    ├── Status events → listen("openclaw-event")
    └── Settings      → invoke("openclaw_get_settings")
```

---

## 13. Appendix: Complete First-Run Timeline

```
User runs `thinclaw` for the first time
│
├── Phase 0: Early Bootstrap
│   ├── dotenvy::dotenv()                    ← Load ./.env (usually doesn't exist)
│   ├── bootstrap::load_thinclaw_env()       ← Load ~/.thinclaw/.env (doesn't exist)
│   └── check_onboard_needed() → "First run"
│
├── Setup Wizard (9 interactive steps)
│   ├── Step 1: Database
│   │   ├── Choose PostgreSQL or libSQL
│   │   ├── Enter connection URL / use default path
│   │   ├── Test connection
│   │   └── Run schema migrations
│   ├── Step 2: Security
│   │   ├── Generate 256-bit master key
│   │   └── Store in OS Keychain (or env var)
│   ├── Step 3: Inference Provider
│   │   ├── Select Anthropic/OpenAI/Ollama/OpenRouter
│   │   └── Enter API key (stored encrypted)
│   ├── Step 4: Model Selection
│   │   └── Pick default model
│   ├── Step 5: Embeddings
│   │   └── Enable semantic search (OpenAI or Ollama)
│   ├── Step 6: Channels
│   │   └── Configure Telegram, Signal, HTTP, Discord, WhatsApp, etc.
│   ├── Step 7: Extensions
│   │   └── Install WASM tools from registry
│   ├── Step 8: Docker Sandbox
│   │   └── Enable sandboxed code execution
│   ├── Step 9: Heartbeat
│   │   └── Configure background task interval
│   └── Writes ONBOARD_COMPLETED=true to ~/.thinclaw/.env
│
├── AppBuilder::build_all()
│   ├── Phase 1: init_database()
│   │   ├── Connect to DB (configured in Step 1)
│   │   ├── Run migrations (idempotent)
│   │   ├── Migrate legacy files if present
│   │   └── Reload config from DB
│   ├── Phase 2: init_secrets()
│   │   ├── Load master key from Keychain (configured in Step 2)
│   │   ├── Create SecretsCrypto
│   │   └── Inject API keys into config
│   ├── Phase 3: init_llm()
│   │   ├── Build provider chain (retry, routing, failover, cache)
│   │   └── Build Provider Vault (runtime-configurable)
│   ├── Phase 4: init_tools()
│   │   ├── Create SafetyLayer
│   │   ├── Register builtin tools + agent control tools
│   │   ├── Create embedding provider
│   │   ├── Create Workspace + MediaPipeline
│   │   ├── Register memory tools
│   │   └── Register subagent tools
│   ├── Phase 5: init_extensions()
│   │   ├── Load WASM tools
│   │   ├── Connect to MCP servers
│   │   ├── Create ExtensionManager
│   │   └── Set up Claude Code delegation
│   └── Post-build:
│       ├── workspace.seed_if_empty()        ← Creates 7 identity files
│       │   ├── README.md
│       │   ├── MEMORY.md     (empty template)
│       │   ├── IDENTITY.md   (name placeholder)
│       │   ├── SOUL.md       (core values)
│       │   ├── AGENTS.md     (session bootstrap)
│       │   ├── USER.md       (user context placeholder)
│       │   └── HEARTBEAT.md  (comment-only seed)
│       └── Backfill embeddings (async)
│
├── Channel Setup
│   ├── Create ChannelManager
│   ├── Add configured channels (REPL, Telegram, Discord, Signal, etc.)
│   ├── Load WASM channels (Telegram, Slack, WhatsApp) + webhook routes
│   └── Set up channel-aware message formatting converters
│
├── Skills Discovery
│   ├── Scan ~/.thinclaw/skills/          (Trusted)
│   ├── Scan <workspace>/skills/          (Trusted)
│   └── Scan <installed>/skills/          (Installed — read-only tools)
│
├── Agent Construction
│   └── Agent::new(config, deps, channels, heartbeat, hygiene, routines, agent_router)
│
├── Web Gateway + Webhook Server (if HTTP channel enabled)
│
└── agent.run()
    ├── Start all channels
    ├── Start background tasks
    │   ├── Self-repair loop
    │   ├── Session pruning (10 min)
    │   ├── Heartbeat (hourly)
    │   ├── Memory hygiene (12h)
    │   ├── Cron routines
    │   ├── Channel health monitor
    │   ├── Zombie reaper (60s)
    │   └── Docker orphan cleanup (once)
    ├── Config file watcher
    ├── Fire BeforeAgentStart hook
    ├── Execute BOOT.md hook (broadcast greeting to preferred channel)
    ├── Execute BOOTSTRAP.md hook (first run only)
    │
    └── Enter message loop
        ├── First message arrives
        │   └── system_prompt_for_context() loads:
        │       AGENTS.md + SOUL.md + USER.md + IDENTITY.md + MEMORY.md
        │       + active channel names
        │       → Assembled into the system prompt
        └── Agent is now fully operational ✅
```

---

## 14. Agent Autonomy — Internal Reasoning & Progress Updates

### Overview

The agent has two built-in control tools designed for long-running, multi-step tasks:

| Tool | Visibility | Purpose |
|------|-----------|---------|
| `agent_think` | **Internal only** (not shown to user) | Reasoning scratchpad — plan, evaluate, decide |
| `emit_user_message` | **Visible to user** (non-terminating) | Send progress updates without ending the loop |

### `agent_think`

Allows the agent to reason without producing user-visible output. The thought is recorded
in the conversation context (so the LLM remembers it), but never forwarded to any channel.

```json
// Tool call
{ "tool": "agent_think", "thought": "I need to read 3 files before deciding. Let me start with config.toml." }

// Tool result (only visible to LLM, not to user)
{ "status": "thought_recorded", "thought": "I need to read 3 files..." }
```

**Source:** `src/tools/builtin/agent_control.rs` — `AgentThinkTool`

### `emit_user_message`

Sends a message to the user through the active channel **without terminating the agent loop**.
The dispatcher intercepts this tool call, extracts the content, sends it via `channels.send_status`,
and then continues the agentic loop as if a normal tool had completed.

```json
// Tool call
{
  "tool": "emit_user_message",
  "content": "I've read all 3 files. Starting the refactor now...",
  "message_type": "progress"
}

// Tool result (LLM sees this, loop continues)
{ "status": "message_sent", "message_type": "progress" }
```

**Message types:** `progress` · `warning` · `question` · `interim_result`

**Source:** `src/tools/builtin/agent_control.rs` — `EmitUserMessageTool`
**Dispatcher interception:** `src/agent/dispatcher.rs` — `run_agentic_loop()`

### Dispatcher Interception

The dispatcher identifies `emit_user_message` calls **before** the normal text-response
termination logic. When detected:

1. Deserializes `content` and `message_type` from the tool result JSON
2. Calls `channels.send_status(StatusUpdate::AgentMessage { ... }, &metadata)`
3. Continues the agentic loop — **the turn does NOT end**

This is the key design: `emit_user_message` is structurally a tool call (not a text response),
so the loop continues until the agent produces a genuine text response.

### Channel Rendering

Each channel renders `AgentMessage` differently:

| Channel | Rendering |
|---------|----------|
| **Web gateway** | SSE `Response` event — appears as a persistent chat message |
| **REPL** | Styled with emoji prefix, rendered as markdown in terminal |
| **Signal** | Sent as a real Signal message with emoji prefix |
| **Slack** | Sent via `chat.postMessage` with emoji prefix |
| **Telegram** | Forwarded by WASM/send_status layer with `[agent_message:]` prefix |
| **Discord** | Sent as a Discord message |
| **WhatsApp** | Sent via Cloud API as a text message |
| **Gmail** | Silently dropped (email is async by nature) |

### Multi-Step Work Pattern

The agent is instructed (via `AGENTS.md` seed) to follow this pattern for complex tasks:

```
1. agent_think("Plan: I'll do X then Y then Z")
2. [execute tool X]
3. agent_think("X worked. Now do Y.")
4. [execute tool Y]
5. emit_user_message("Halfway done — X and Y complete. Starting Z...")  ← user sees this
6. [execute tool Z]
7. agent_think("All done. Compose final response.")
8. [produce text response]  ← turn ends here
```

**Key rule:** A text response ends the agent's turn. Tools (including `emit_user_message`)
do not. So the agent should keep calling tools as long as it has work to do.

### Protected Tools

Both tools are added to `PROTECTED_TOOL_NAMES` in the registry — they cannot be
shadowed or overridden by dynamically loaded WASM tools or MCP extensions.

**Source:** `src/tools/registry.rs` — `PROTECTED_TOOL_NAMES`
