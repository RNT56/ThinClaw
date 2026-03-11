<p align="center">
  <img src="ironclaw.png?v=2" alt="IronClaw" width="200"/>
</p>

<h1 align="center">IronClaw</h1>

<p align="center">
  <strong>Your secure personal AI assistant, always on your side</strong>
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
  <a href="https://t.me/ironclawAI"><img src="https://img.shields.io/badge/Telegram-%40ironclawAI-26A5E4?style=flat&logo=telegram&logoColor=white" alt="Telegram: @ironclawAI" /></a>
  <a href="https://www.reddit.com/r/ironclawAI/"><img src="https://img.shields.io/badge/Reddit-r%2FironclawAI-FF4500?style=flat&logo=reddit&logoColor=white" alt="Reddit: r/ironclawAI" /></a>
</p>

<p align="center">
  <a href="#philosophy">Philosophy</a> •
  <a href="#features">Features</a> •
  <a href="#llm-providers">LLM Providers</a> •
  <a href="#installation">Installation</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a>
</p>

---

## Philosophy

IronClaw is built on a simple principle: **your AI assistant should work for you, not against you**.

In a world where AI systems are increasingly opaque about data handling and aligned with corporate interests, IronClaw takes a different approach:

- **Your data stays yours** — All information is stored locally, encrypted, and never leaves your control
- **Transparency by design** — Open source, auditable, no hidden telemetry or data harvesting
- **Self-expanding capabilities** — Build new tools on the fly without waiting for vendor updates
- **Defense in depth** — Multiple security layers protect against prompt injection and data exfiltration
- **Bring your own model** — Works with any LLM provider, no vendor lock-in

IronClaw is the AI assistant you can actually trust with your personal and professional life.

## Features

### 🔒 Security First

- **WASM Sandbox** — Untrusted tools run in isolated WebAssembly containers with capability-based permissions
- **Credential Protection** — Secrets are never exposed to tools; injected at the host boundary with leak detection
- **Prompt Injection Defense** — Pattern detection, content sanitization, and policy enforcement
- **Endpoint Allowlisting** — HTTP requests only to explicitly approved hosts and paths

### 🌐 Always Available

- **Multi-channel** — REPL, HTTP webhooks, WASM channels (Telegram, Slack, Discord, WhatsApp), Nostr, Signal, and web gateway
- **Web Gateway** — Browser UI with real-time SSE/WebSocket streaming, chat threads, memory browser, and extension management
- **Docker Sandbox** — Isolated container execution with per-job tokens and orchestrator/worker pattern
- **Routines** — Cron schedules, event triggers, webhook handlers for background automation
- **Heartbeat System** — Proactive background execution for monitoring and maintenance tasks
- **Parallel Jobs** — Handle multiple requests concurrently with isolated contexts
- **Self-repair** — Automatic detection and recovery of stuck operations

### 🧩 Self-Expanding

- **Dynamic Tool Building** — Describe what you need, and IronClaw builds it as a WASM tool
- **MCP Protocol** — Connect to Model Context Protocol servers for additional capabilities
- **Plugin Architecture** — Drop in new WASM tools and channels without restarting
- **Extension Registry** — Install, update, and manage tools from a curated catalog
- **Skills Framework** — Structured multi-step procedures with frontmatter metadata

### 🧠 Persistent Memory

- **Hybrid Search** — Full-text + vector (semantic) search using Reciprocal Rank Fusion
- **Workspace Filesystem** — Flexible path-based storage for notes, logs, and context
- **Identity Files** — Maintain consistent personality and preferences across sessions
- **Context Management** — Automatic context window management with intelligent summarization

### 🎙️ Hardware Bridge

- **Sensor Access** — Camera, microphone, and screen capture via host-provided bridge (Scrappy desktop)
- **3-Tier Approval** — Deny / Allow Once / Allow Session permission model for hardware access
- **Voice Wake** — Wake word detection for hands-free activation (headless mode)
- **Talk Mode** — Continuous voice conversation via STT + TTS pipeline

## LLM Providers

IronClaw works with **any OpenAI-compatible endpoint** out of the box. The default backend is `OpenAiCompatible`, giving you maximum flexibility:

| Provider | Backend Value | Required Env Vars |
|----------|--------------|-------------------|
| **OpenRouter** (300+ models) | `openai_compatible` | `LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL` |
| **Anthropic** (Claude) | `anthropic` | `ANTHROPIC_API_KEY` |
| **OpenAI** (GPT) | `openai` | `OPENAI_API_KEY` |
| **Ollama** (local) | `ollama` | `OLLAMA_BASE_URL` (optional) |
| **Tinfoil** (private inference) | `tinfoil` | `TINFOIL_API_KEY` |
| **Together AI** | `openai_compatible` | `LLM_BASE_URL`, `LLM_API_KEY` |
| **Fireworks AI** | `openai_compatible` | `LLM_BASE_URL`, `LLM_API_KEY` |
| **vLLM / LiteLLM** (self-hosted) | `openai_compatible` | `LLM_BASE_URL` |

**Quick start with OpenRouter:**

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

### Reliability Features

All backends support these optional reliability settings:

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| Retries | `LLM_MAX_RETRIES` | 3 | Retry transient failures with backoff |
| Circuit breaker | `CIRCUIT_BREAKER_THRESHOLD` | disabled | Open circuit after N consecutive failures |
| Failover | `LLM_FALLBACK_MODEL` | none | Automatic fallback to a secondary model |
| Response cache | `RESPONSE_CACHE_ENABLED` | false | Cache repeated prompts to save tokens |
| Cheap model | `LLM_CHEAP_MODEL` | none | Fast model for lightweight tasks (heartbeat, routing) |

See [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md) for a full provider guide.

## Installation

### Prerequisites

- Rust 1.92+
- One of:
  - **PostgreSQL 15+** with [pgvector](https://github.com/pgvector/pgvector) extension
  - **libSQL** (embedded, zero dependencies) with optional [Turso](https://turso.tech) cloud sync

## Download or Build

Visit the [Releases page](https://github.com/nearai/ironclaw/releases/) to see the latest updates.

<details>
  <summary>Install via Windows Installer (Windows)</summary>

Download the [Windows Installer](https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-x86_64-pc-windows-msvc.msi) and run it.

</details>

<details>
  <summary>Install via PowerShell script (Windows)</summary>

```sh
irm https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.ps1 | iex
```

</details>

<details>
  <summary>Install via shell script (macOS, Linux, Windows/WSL)</summary>

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```
</details>

<details>
  <summary>Install via Homebrew (macOS/Linux)</summary>

```sh
brew install ironclaw
```

</details>

<details>
  <summary>Compile from source (Cargo on Windows, Linux, macOS)</summary>

Install it with `cargo`, just make sure you have [Rust](https://rustup.rs) installed on your computer.

```bash
# Clone the repository
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# Build (full features — PostgreSQL + libSQL + all runtime modules)
cargo build --release

# Build (desktop embedding — libSQL only, minimal footprint)
cargo build --release --no-default-features --features desktop

# Run tests
cargo test
```

For **full release** (after modifying channel sources), run `./scripts/build-all.sh` to rebuild channels first.

</details>

### Database Setup

**Option A: PostgreSQL** (production, requires running server)

```bash
createdb ironclaw
psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

**Option B: libSQL** (embedded, zero dependencies)

No setup needed — the database file is created automatically at `~/.ironclaw/ironclaw.db` on first run.

For Turso cloud sync, set `LIBSQL_URL` and `LIBSQL_AUTH_TOKEN`.

## Configuration

Run the setup wizard to configure IronClaw:

```bash
ironclaw onboard
```

The wizard guides you through:

1. **Database** — PostgreSQL or libSQL connection
2. **Security** — Secrets master key (OS keychain or environment variable)
3. **Inference provider** — Anthropic, OpenAI, Ollama, OpenRouter, or any OpenAI-compatible endpoint
4. **Model selection** — Pick from available models or enter a custom model ID
5. **Embeddings** — Semantic search via OpenAI or Ollama embeddings
6. **Channels** — CLI, HTTP, Signal, Nostr, Telegram, and web gateway
7. **Extensions** — Install tools from the registry
8. **Docker sandbox** — Isolated container execution
9. **Background tasks** — Heartbeat and routine scheduling

Settings are persisted in the connected database. Bootstrap variables (e.g. `DATABASE_URL`, `LLM_BACKEND`)
are written to `~/.ironclaw/.env` so they are available before the database connects.

### Environment Variables

You can also configure IronClaw directly via environment variables or a `.env` file:

```env
# Database (choose one)
DATABASE_BACKEND=postgres          # or: libsql
DATABASE_URL=postgres://user:pass@localhost/ironclaw

# LLM Provider
LLM_BACKEND=openai_compatible     # or: anthropic, openai, ollama, tinfoil
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4

# Embeddings
EMBEDDING_ENABLED=true
EMBEDDING_PROVIDER=openai          # or: ollama
OPENAI_API_KEY=sk-...

# Web Gateway
GATEWAY_ENABLED=true
GATEWAY_PORT=3000
```

## Security

IronClaw implements defense in depth to protect your data and prevent misuse.

### WASM Sandbox

All untrusted tools run in isolated WebAssembly containers:

- **Capability-based permissions** — Explicit opt-in for HTTP, secrets, tool invocation
- **Endpoint allowlisting** — HTTP requests only to approved hosts/paths
- **Credential injection** — Secrets injected at host boundary, never exposed to WASM code
- **Leak detection** — Scans requests and responses for secret exfiltration attempts
- **Rate limiting** — Per-tool request limits to prevent abuse
- **Resource limits** — Memory, CPU, and execution time constraints

```
WASM ──► Allowlist ──► Leak Scan ──► Credential ──► Execute ──► Leak Scan ──► WASM
         Validator     (request)     Injector       Request     (response)
```

### Prompt Injection Defense

External content passes through multiple security layers:

- Pattern-based detection of injection attempts
- Content sanitization and escaping
- Policy rules with severity levels (Block/Warn/Review/Sanitize)
- Tool output wrapping for safe LLM context injection

### Data Protection

- All data stored locally in your database (PostgreSQL or libSQL)
- Secrets encrypted with AES-256-GCM (or OS keychain on macOS/Linux)
- No telemetry, analytics, or data sharing
- Full audit log of all tool executions

## Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                          Channels                              │
│  ┌──────┐  ┌──────┐   ┌─────────────┐  ┌─────────────┐         │
│  │ REPL │  │ HTTP │   │WASM Channels│  │ Web Gateway │         │
│  └──┬───┘  └──┬───┘   │  Telegram   │  │ (SSE + WS)  │         │
│     │         │        │  Slack      │  └──────┬──────┘         │
│     │         │        │  Discord    │         │                │
│     │         │        └──────┬──────┘         │                │
│     └─────────┴──────────────┴─────────────────┘                │
│                              │                                 │
│                    ┌─────────▼─────────┐                       │
│                    │    Agent Loop     │  Intent routing       │
│                    └────┬──────────┬───┘                       │
│                         │          │                           │
│              ┌──────────▼────┐  ┌──▼───────────────┐           │
│              │  Scheduler    │  │ Routines Engine  │           │
│              │(parallel jobs)│  │(cron, event, wh) │           │
│              └──────┬────────┘  └────────┬─────────┘           │
│                     │                    │                     │
│       ┌─────────────┼────────────────────┘                     │
│       │             │                                          │
│   ┌───▼─────┐  ┌────▼────────────────┐                         │
│   │ Local   │  │    Orchestrator     │                         │
│   │Workers  │  │  ┌───────────────┐  │                         │
│   │(in-proc)│  │  │ Docker Sandbox│  │                         │
│   └───┬─────┘  │  │   Containers  │  │                         │
│       │        │  └───────────────┘  │                         │
│       │        └─────────┬───────────┘                         │
│       └──────────────────┤                                     │
│                          │                                     │
│              ┌───────────▼──────────┐                          │
│              │    Tool Registry     │                          │
│              │  Built-in, MCP, WASM │                          │
│              └──────────────────────┘                          │
└────────────────────────────────────────────────────────────────┘
```

### Core Components

| Component | Purpose |
|-----------|---------|
| **Agent Loop** | Main message handling and job coordination |
| **Router** | Classifies user intent (command, query, task) |
| **Scheduler** | Manages parallel job execution with priorities |
| **Worker** | Executes jobs with LLM reasoning and tool calls |
| **Orchestrator** | Container lifecycle, LLM proxying, per-job auth |
| **Web Gateway** | Browser UI with chat, memory, jobs, logs, extensions, routines |
| **Routines Engine** | Scheduled (cron) and reactive (event, webhook) background tasks |
| **Workspace** | Persistent memory with hybrid full-text + vector search |
| **Safety Layer** | Prompt injection defense, leak detection, and content sanitization |
| **Skill Registry** | Structured multi-step procedures with discovery and execution |
| **Hardware Bridge** | Sensor access (camera, mic, screen) with per-request approval |
| **Cost Guard** | Token usage tracking and configurable spending limits |

### Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `full` | Everything (PostgreSQL + libSQL + all modules) | ✅ |
| `desktop` | Tauri embedding (libSQL only, minimal footprint) | |
| `postgres` | PostgreSQL + pgvector support | ✅ |
| `libsql` | Embedded libSQL/Turso support | ✅ |
| `html-to-markdown` | Web page → markdown conversion | ✅ |
| `repl` | Interactive terminal REPL + boot screen | ✅ |
| `voice` | Voice wake word detection (cpal audio capture) | |

## Usage

```bash
# First-time setup (configures database, providers, channels)
ironclaw onboard

# Start interactive REPL
ironclaw

# Single message mode
ironclaw --message "What's the weather in Berlin?"

# With debug logging
RUST_LOG=ironclaw=debug ironclaw

# Run diagnostics
ironclaw doctor

# Check system status
ironclaw status
```

## Development

```bash
# Format code
cargo fmt

# Lint
cargo clippy --all --benches --tests --examples --all-features

# Run tests
createdb ironclaw_test
cargo test

# Run specific test
cargo test test_name
```

- **Telegram channel**: See [docs/TELEGRAM_SETUP.md](docs/TELEGRAM_SETUP.md) for setup and DM pairing.
- **Changing channel sources**: Run `./channels-src/telegram/build.sh` before `cargo build` so the updated WASM is bundled.

## OpenClaw Heritage

IronClaw is a Rust reimplementation inspired by [OpenClaw](https://github.com/openclaw/openclaw). See [FEATURE_PARITY.md](FEATURE_PARITY.md) for the complete tracking matrix.

Key differences:

- **Rust vs TypeScript** — Native performance, memory safety, single binary
- **WASM sandbox vs Docker** — Lightweight, capability-based security
- **PostgreSQL + libSQL** — Flexible storage from embedded to production
- **Security-first design** — Multiple defense layers, credential protection
- **Provider agnostic** — Works with any OpenAI-compatible endpoint

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
