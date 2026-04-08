<p align="center">
  <img src="Thinclaw_IC_01_nobg.png" alt="ThinClaw" width="200"/>
</p>

<h1 align="center">ThinClaw</h1>

<p align="center">
  <strong>A hardened, self-expanding AI agent you actually control</strong>
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
  <a href="https://github.com/RNT56/ThinClaw/releases"><img src="https://img.shields.io/github/v/release/RNT56/ThinClaw?color=green&label=release" alt="Latest Release" /></a>
  <a href="https://github.com/RNT56/ThinClaw/actions"><img src="https://img.shields.io/github/actions/workflow/status/RNT56/ThinClaw/ci.yml?branch=main&label=CI" alt="CI" /></a>
</p>

<p align="center">
  <a href="#what-is-thinclaw">What is ThinClaw?</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#features">Features</a> •
  <a href="#llm-providers">LLM Providers</a> •
  <a href="#installation">Installation</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#web-ui">Web UI</a> •
  <a href="#remote-hosting">Remote Hosting</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a>
</p>

---

## What is ThinClaw?

ThinClaw is a **production-grade personal AI agent** written in Rust. It runs on your own hardware — a Mac Mini, a Linux VPS, or your laptop — and connects to whatever LLM you prefer. It is not a chat wrapper. It is a full agent runtime: multi-channel, tool-using, memory-equipped, and security-hardened.

**Why ThinClaw?**

- **Your data, your hardware** — nothing leaves your control. All memory and conversation history is stored in your own database (libSQL or PostgreSQL).
- **Any model, any provider** — native Anthropic, OpenAI, Gemini, Ollama, and any OpenAI-compatible endpoint. Switch models mid-conversation without restarting.
- **Built for real use** — cost tracking with hard limits, smart model routing, cron-based routines, webhook automation, and multi-channel delivery (Telegram, Discord, iMessage, Signal, and more).
- **Extensible by design** — WASM tools and channels run in a sandboxed runtime. Add new capabilities without touching core code or restarting.
- **Skills framework** — structured multi-step procedures with hot-reload, proactive discovery, and workspace-level overrides.

ThinClaw is the server-side Rust engine behind the [Scrappy](https://github.com/RNT56/scrappy) desktop app, and it runs equally well as a standalone headless agent on a server you SSH into.

---

## Quick Start

```bash
# 1. Install (macOS / Linux / WSL)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh

# 2. Run the setup wizard (database, LLM key, channels)
thinclaw onboard

# 3. Start the agent (REPL mode)
thinclaw

# 4. Or run headless with the web UI on port 3000
thinclaw run --no-onboard
# → Open http://localhost:3000 in your browser
```

**Minimum to get started:** one LLM API key (Anthropic, OpenAI, or OpenRouter). No other dependencies for the libSQL build.

---

## Features

| Area | Highlights |
|------|------------|
| **Agent Runtime** | Parallel jobs, subagent orchestration, A2A messaging, cron routines, heartbeat, self-repair |
| **Security** | WASM sandbox, credential protection, prompt injection defense, Docker isolation, shell sandbox |
| **Cost Control** | Cost Guard with hard limits, smart model routing, failover, routing policy, live cost dashboard |
| **Channels** | Telegram, Discord, Signal, WhatsApp, iMessage, Gmail, Apple Mail, Nostr, HTTP, Web Gateway |
| **Extensibility** | Skills framework with hot-reload, dynamic WASM tool building, MCP protocol, ClawHub registry |
| **Memory** | Hybrid full-text + vector search (RRF), workspace filesystem, identity files, context compaction |
| **Hardware** | Camera, microphone, screen capture, voice wake, talk mode (Scrappy desktop bridge) |

### Agent Runtime

- **Parallel jobs** — multiple requests handled concurrently with isolated contexts and priority scheduling
- **Subagent orchestration** — spawn parallel sub-agents with tool filtering, iteration caps, and timeout enforcement
- **Agent-to-Agent (A2A)** — persistent multi-agent registry; one agent can message another via the `message_agent` tool
- **Routines engine** — cron schedules, event triggers, and webhook handlers for background automation
- **Heartbeat system** — periodic background execution with self-critique feedback loop and stuck-heartbeat recovery
- **Self-repair** — automatic detection and recovery of stuck operations; `thinclaw doctor` for diagnostics

### Security

- **WASM sandbox** — untrusted tools run in isolated WebAssembly containers with capability-based permissions
- **Credential protection** — secrets are never exposed to tools; injected at the host boundary with outbound leak detection
- **Prompt injection defense** — pattern detection, content sanitization, severity-graded policy enforcement
- **Endpoint allowlisting** — HTTP requests only to explicitly approved hosts and paths
- **Docker sandbox** — isolated container execution with per-job ephemeral tokens, dropped capabilities, and non-root execution
- **Shell sandbox** — 3-layer command validation: safe-bins allowlist, workdir confinement, path-traversal detection

### Cost Control

- **Cost Guard** — real-time token and spend tracking per model and per agent, with configurable daily and hard limits
- **Smart routing** — route lightweight tasks (heartbeats, evaluations) to a cheap model automatically
- **Multi-provider failover** — automatic failover across configured providers with cooldown management
- **Routing Policy** — fine-grained rules for model selection by task type, cost target, or capability
- **Live cost dashboard** — WebUI Costs tab with daily spend charts, model breakdown, budget progress, and CSV export

### Channels

- **Multi-channel** — REPL, HTTP webhooks, WASM channels (Telegram, Slack, Discord, WhatsApp), Nostr, Signal, Gmail, Apple Mail, iMessage, and web gateway
- **Web Gateway** — browser UI with real-time SSE/WebSocket streaming, chat threads, memory browser, costs, jobs, logs, and settings
- **OpenAI-compatible API** — `/v1/chat/completions` endpoint; point any OpenAI SDK at ThinClaw
- **Managed tunnels** — ngrok, Cloudflare Tunnel, and Tailscale for public webhook delivery
- **Standalone or embedded** — headless server binary or embedded inside the Scrappy desktop app (Tauri)

### Extensibility

- **Skills framework** — structured multi-step procedures with frontmatter, trust gating, proactive mid-turn discovery, and hot-reload without restart
- **Dynamic tool building** — describe what you need; ThinClaw builds it as a WASM tool
- **MCP protocol** — connect to Model Context Protocol servers (OAuth 2.1, stdio + HTTP transport)
- **Plugin architecture** — drop in new WASM tools and channels; watched directory for hot-loading
- **ClawHub** — extension marketplace to discover, install, and manage tools from a curated catalog

### Memory

- **Hybrid search** — full-text + vector (semantic) search using Reciprocal Rank Fusion with temporal decay and MMR re-ranking
- **Workspace filesystem** — flexible path-based storage for notes, logs, and context
- **Identity files** — `AGENTS.md`, `BOOT.md`, `BOOTSTRAP.md` for consistent personality and startup behaviour across sessions
- **Context management** — automatic window management with intelligent summarisation and post-compaction read audit

### Hardware Bridge

- **Sensor access** — camera, microphone, and screen capture via host-provided bridge (Scrappy desktop)
- **3-tier approval** — Deny / Allow Once / Allow Session permission model for hardware access
- **Voice wake** — VAD + Sherpa-ONNX keyword spotting for hands-free activation
- **Talk mode** — continuous voice conversation via STT + TTS pipeline

---

## LLM Providers

ThinClaw works with **any OpenAI-compatible endpoint** out of the box. The default backend is `openai_compatible`, giving you maximum flexibility:

| Provider | Backend Value | Required Env Vars |
|----------|--------------|-------------------|
| **OpenRouter** (300+ models) | `openai_compatible` | `LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL` |
| **Anthropic** (Claude) | `anthropic` | `ANTHROPIC_API_KEY` |
| **OpenAI** (GPT) | `openai` | `OPENAI_API_KEY` |
| **Google Gemini** | `gemini` | `GEMINI_API_KEY` |
| **AWS Bedrock** | `bedrock` | `BEDROCK_API_KEY`, `AWS_REGION` |
| **Ollama** (local) | `ollama` | `OLLAMA_BASE_URL` (optional) |
| **llama.cpp** (local) | `llama_cpp` | `LLAMA_CPP_SERVER_URL` |
| **LM Studio** (local) | `openai_compatible` | `LLM_BASE_URL` |
| **Tinfoil** (private inference) | `tinfoil` | `TINFOIL_API_KEY` |
| **Groq / Together / Fireworks** | `openai_compatible` | `LLM_BASE_URL`, `LLM_API_KEY` |
| **vLLM / LiteLLM** (self-hosted) | `openai_compatible` | `LLM_BASE_URL` |

**Quick start with OpenRouter:**

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

### Smart Routing & Reliability

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| Smart routing | `LLM_ROUTING_ENABLED` | `false` | Enable routing policy engine |
| Cheap model | `LLM_CHEAP_MODEL` | none | Route lightweight tasks to a cheaper model |
| Fallback | `LLM_FALLBACK_MODEL` | none | Automatic failover on provider errors |
| Retries | `LLM_MAX_RETRIES` | 3 | Retry transient failures with backoff |
| Circuit breaker | `CIRCUIT_BREAKER_THRESHOLD` | disabled | Open after N consecutive failures |
| Response cache | `RESPONSE_CACHE_ENABLED` | false | Cache repeated prompts to save tokens |

See [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md) for the full provider guide.

---

## Installation

### Prerequisites

**Pre-built binary** (via installer script or Releases page): No prerequisites — the binary is self-contained.

**Compile from source:**

| Prerequisite | Purpose | Install |
|---|---|---|
| Xcode CLI Tools (macOS) | C compiler, linker | `xcode-select --install` |
| build-essential + libssl-dev (Linux) | C compiler, OpenSSL | `sudo apt install build-essential pkg-config libssl-dev` |
| **Rust 1.92+** | Rust compiler | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **wasm32-wasip2 target** | WASM compilation | `rustup target add wasm32-wasip2` |
| **wasm-tools** | WASM component model | `cargo install wasm-tools --locked` |
| **cargo-component** | Build WASM extensions | `cargo install cargo-component --locked` |

**One-click setup** (installs all prerequisites automatically):

```bash
# macOS — full deploy (installs everything, builds, launches):
./scripts/mac-deploy.sh

# Developer setup (any OS — installs WASM toolchain, runs tests):
./scripts/dev-setup.sh
```

**Database** (choose one):
- **libSQL** (embedded, zero dependencies) — recommended for single-server / Mac Mini
- **PostgreSQL 15+** with [pgvector](https://github.com/pgvector/pgvector) — production multi-server

## Download or Build

Visit the [Releases page](https://github.com/RNT56/ThinClaw/releases/) to see the latest updates.

<details>
  <summary>Install via Windows Installer (Windows)</summary>

Download the [Windows Installer](https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-x86_64-pc-windows-msvc.msi) and run it.

</details>

<details>
  <summary>Install via PowerShell script (Windows)</summary>

```sh
irm https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.ps1 | iex
```

</details>

<details>
  <summary>Install via shell script (macOS, Linux, Windows/WSL)</summary>

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh
```
</details>

<details>
  <summary>Install via Homebrew (macOS/Linux)</summary>

```sh
brew install thinclaw
```

</details>

<details>
  <summary>Compile from source (Cargo on Windows, Linux, macOS)</summary>

```bash
# Clone the repository
git clone https://github.com/RNT56/ThinClaw.git
cd ThinClaw

# Build (light profile — default: databases + document extraction)
cargo build --release

# Build (full features — light + web gateway + REPL + tunnel + Docker)
cargo build --release --features full

# Build (desktop embedding — libSQL only, minimal footprint)
cargo build --release --features desktop

# Build (air-gapped — all WASM extensions embedded in binary, +6-13 MB)
cargo build --release --features full,bundled-wasm

# Run tests
cargo test
```

For **full release** (after modifying channel sources), run `./scripts/build-all.sh` to rebuild channels first.

For an **air-gapped build** with all WASM extensions embedded:
```bash
./scripts/build-all.sh --bundled
```

</details>

### Database Setup

**Option A: PostgreSQL** (production, requires running server)

```bash
createdb thinclaw
psql thinclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

**Option B: libSQL** (embedded, zero dependencies)

No setup needed — the database file is created automatically at `~/.thinclaw/thinclaw.db` on first run.

For Turso cloud sync, set `LIBSQL_URL` and `LIBSQL_AUTH_TOKEN`.

---

## Configuration

Run the setup wizard to configure ThinClaw:

```bash
thinclaw onboard
```

The wizard guides you through:

1. **Database** — PostgreSQL or libSQL connection
2. **Security** — Secrets master key (OS keychain or environment variable)
3. **Inference provider** — Anthropic, OpenAI, Ollama, OpenRouter, or any OpenAI-compatible endpoint
4. **Model selection** — Pick from available models or enter a custom model ID
5. **Smart routing** — Optional cheap model for lightweight tasks + fallback provider
6. **Embeddings** — Semantic search via OpenAI, Gemini, or Ollama embeddings
7. **Channels** — CLI, HTTP, Signal, Nostr, Telegram, Discord, Apple Mail, iMessage, and web gateway
8. **Extensions** — Install tools from the registry
9. **Docker sandbox** — Isolated container execution (Claude Code, Rust, Node, Python)
10. **Background tasks** — Heartbeat interval and notification routing
11. **Timezone** — Agent-local timezone for accurate cron scheduling
12. **Notification preferences** — Preferred channel for agent-initiated messages

Settings are persisted in the connected database. Bootstrap variables (e.g. `DATABASE_URL`, `LLM_BACKEND`) are written to `~/.thinclaw/.env` so they are available before the database connects.

> **Upgrading & Reset:** Rebuilding with `cargo build --release` only updates the binary — all data in `~/.thinclaw/` is untouched. To fully reset, remove `~/.thinclaw/` and re-run. See [Data Directory, Upgrades & Reset](docs/DEPLOYMENT.md#data-directory-upgrades--reset) for the full directory layout and partial reset options.

### Environment Variables

You can also configure ThinClaw directly via environment variables or a `.env` file:

```env
# Database (choose one)
DATABASE_BACKEND=libsql             # or: postgres
LIBSQL_PATH=~/.thinclaw/thinclaw.db
# DATABASE_URL=postgres://user:pass@localhost/thinclaw

# LLM Provider
LLM_BACKEND=openai_compatible       # or: anthropic, openai, ollama, gemini, tinfoil
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4

# Smart routing (optional)
LLM_ROUTING_ENABLED=true
LLM_CHEAP_MODEL=openai/gpt-4o-mini
LLM_FALLBACK_MODEL=openai/gpt-4o

# Embeddings
EMBEDDING_ENABLED=true
EMBEDDING_PROVIDER=openai           # or: ollama, gemini
OPENAI_API_KEY=sk-...

# Web Gateway
GATEWAY_ENABLED=true
GATEWAY_HOST=127.0.0.1              # use 0.0.0.0 for remote access
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=<run: openssl rand -hex 32>

# Agent
AGENT_NAME=my-agent
AGENT_TIMEZONE=Europe/Berlin        # IANA timezone for cron scheduling

# Cost tracking
COST_BUDGET_DAILY_USD=5.00          # soft daily budget (triggers warning)
COST_HARD_LIMIT_USD=20.00           # hard monthly limit (blocks LLM calls)

# Channels (all optional)
TELEGRAM_BOT_TOKEN=...
TELEGRAM_OWNER_ID=...
DISCORD_BOT_TOKEN=...
SIGNAL_HTTP_URL=http://localhost:8080
IMESSAGE_ALLOW_FROM=+1234567890     # macOS only
```

**Full environment variable reference:** [docs/DEPLOYMENT.md#environment-reference](docs/DEPLOYMENT.md#environment-reference)

---

## Web UI

When the `web-gateway` feature is enabled (`--features full`), ThinClaw serves a browser UI at `http://localhost:3000` (or your configured `GATEWAY_HOST:GATEWAY_PORT`).

### What the Web UI includes

| Tab | Description |
|-----|-------------|
| **Chat** | Real-time conversation with SSE/WebSocket streaming, thread history |
| **Memory** | Browse, search, and edit the agent's workspace and identity files |
| **Jobs** | Live status of running and completed jobs with log tailing |
| **Costs** | Daily spend charts, per-model breakdown, budget progress, CSV export |
| **Routines** | Create and manage cron jobs, event triggers, and webhook automations |
| **Providers** | Add API keys, configure smart routing, drag-and-drop model pools |
| **Settings** | All agent settings grouped by section (channels, safety, heartbeat, etc.) |
| **Extensions** | Install, enable, and hot-reload WASM tools and channels |
| **Skills** | List, install, and hot-reload skills from workspace or registry |
| **Logs** | Real-time log stream with level and target filters |

### OpenAI-Compatible API

ThinClaw exposes a drop-in OpenAI-compatible endpoint — point any OpenAI SDK at it:

```python
import openai
client = openai.OpenAI(
    base_url="http://localhost:3000/v1",
    api_key="<your GATEWAY_AUTH_TOKEN>"
)
response = client.chat.completions.create(
    model="thinclaw",
    messages=[{"role": "user", "content": "Hello"}]
)
```

---

## Remote Hosting

ThinClaw is designed to run as a **standalone headless agent** on a dedicated server (Mac Mini, VPS, etc.) and connect to the [Scrappy](https://github.com/RNT56/scrappy) desktop app for remote control.

### Quick Remote Setup

```bash
# On your server (Mac Mini / Linux VPS)
git clone https://github.com/RNT56/ThinClaw.git && cd ThinClaw
cargo build --release --features libsql
./target/release/thinclaw   # runs onboarding wizard on first launch
```

Enable the **Gateway** channel during setup (port 3000 or 18789, host `0.0.0.0`), then connect Scrappy via **Settings → Gateway → Connect Existing**.

### Deployment Options

| Method | Best For | Guide |
|--------|----------|-------|
| Direct binary | Mac Mini, macOS/Linux servers | [docs/DEPLOYMENT.md — Path 1](docs/DEPLOYMENT.md#path-1-direct-binary-recommended-for-mac-mini) |
| Docker Compose | Linux VPS, cloud servers | [docs/DEPLOYMENT.md — Path 2](docs/DEPLOYMENT.md#path-2-docker-compose-any-os) |
| Scrappy one-click SSH deploy | Linux servers | [docs/DEPLOYMENT.md — Path 3](docs/DEPLOYMENT.md#path-3-automated-deploy-from-scrappy-linux-targets) |
| Linux server bootstrap script | Fresh VPS (Docker + UFW + Fail2ban + Tailscale) | [docs/DEPLOYMENT.md — Path 4](docs/DEPLOYMENT.md#path-4-automated-linux-server-setup) |

### Securing the Connection

> **Note:** The gateway uses plain HTTP by default. Do not expose port 3000/18789 to the public internet without encryption.

**Recommended: Tailscale (free mesh VPN)**

```bash
# Install on both machines (server + your laptop)
brew install tailscale          # macOS
curl -fsSL https://tailscale.com/install.sh | sh   # Linux

# Authenticate
sudo tailscale up

# Get the server's Tailscale IP
tailscale ip -4   # e.g. 100.64.0.2

# Configure ThinClaw to bind to Tailscale only
GATEWAY_HOST=100.64.0.2

# In Scrappy: Settings → Gateway → http://100.64.0.2:3000
```

**Alternative: nginx reverse proxy with Let's Encrypt SSL**

See [docs/DEPLOYMENT.md — Securing the Connection](docs/DEPLOYMENT.md#securing-the-connection-tailscale) for the full nginx config.

### Auto-Start on Boot

```bash
thinclaw                   # Complete the onboarding wizard first
thinclaw service install   # Creates launchd plist (macOS) or systemd unit (Linux)
thinclaw service start     # Starts the agent in the background
thinclaw service status    # Check if it's running
```

The service runs `thinclaw run --no-onboard` with **all configured channels**. launchd/systemd automatically restarts ThinClaw if it crashes.

See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for the full deployment guide.

---

## Usage

```bash
# First-time setup (configures database, providers, channels)
thinclaw onboard

# Start interactive REPL
thinclaw

# Single message mode
thinclaw --message "What's the weather in Berlin?"

# With debug logging
RUST_LOG=thinclaw=debug thinclaw

# Run diagnostics
thinclaw doctor

# Check system status
thinclaw status

# Manage channels
thinclaw channels list

# List and test models
thinclaw models list
thinclaw models test

# Memory management
thinclaw memory search "meeting notes"
thinclaw memory tree

# Skills
thinclaw skills list
thinclaw skills install <name>

# Service management
thinclaw service install
thinclaw service start | stop | status | uninstall

# Update ThinClaw
thinclaw update check
thinclaw update install
```

---

## Security

ThinClaw implements **defense in depth** to protect your data and prevent misuse.

### WASM Sandbox

All untrusted tools run in isolated WebAssembly containers:

- **Capability-based permissions** — Explicit opt-in for HTTP, secrets, tool invocation
- **Endpoint allowlisting** — HTTP requests only to approved hosts/paths
- **Credential injection** — Secrets injected at host boundary, never exposed to WASM code
- **Leak detection** — Scans both requests and responses for secret exfiltration
- **Key rotation** — Automated credential rotation with configurable policies
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
- Policy rules with severity levels (Block / Warn / Review / Sanitize)
- Tool output wrapping for safe LLM context injection

### Docker Sandbox

ThinClaw can run untrusted jobs in isolated Docker containers with a full orchestrator/worker architecture:

- **Per-job auth tokens** — Each container gets a unique ephemeral token, revoked on completion
- **Bind-mount validation** — Project directories restricted to `~/.thinclaw/projects/` with canonicalization checks
- **Capabilities dropped** — All Linux capabilities except `CHOWN` removed
- **Non-root execution** — Runs as UID 1000 (`sandbox` user) with `no-new-privileges`
- **Resource limits** — Configurable memory (default 2 GB) and CPU shares
- **HEALTHCHECK** — Built-in container health monitoring with automatic orphan cleanup on restart

**Building the worker image:**

```bash
# Required before first use (the wizard offers to do this automatically)
docker build -f Dockerfile.worker -t thinclaw-worker .
```

The worker image includes: Rust 1.92, Node.js, Python 3, Git, GitHub CLI, and the Claude Code CLI.

### Claude Code Sandbox

ThinClaw can delegate complex coding tasks to Anthropic's [Claude Code](https://docs.anthropic.com/en/docs/claude-code) CLI, running inside a sandboxed Docker container.

**Authentication** (checked in priority order):

| Priority | Source | How to set |
|----------|--------|------------|
| 1 | `ANTHROPIC_API_KEY` env var | `export ANTHROPIC_API_KEY=sk-ant-api03-...` |
| 2 | OS keychain | Set during `thinclaw onboard` |
| 3 | Claude Code OAuth | Run `claude login` on the host machine |

**Configuration:**

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| Enabled | `CLAUDE_CODE_ENABLED` | `false` | Enable Claude Code as a delegation target |
| Model | `CLAUDE_CODE_MODEL` | `sonnet` | Claude model for code tasks |
| Max turns | `CLAUDE_CODE_MAX_TURNS` | `50` | Maximum agentic turns per job |
| Memory | `CLAUDE_CODE_MEMORY_LIMIT_MB` | `4096` | Container memory limit |

### Data Protection

- All data stored locally in your database (PostgreSQL or libSQL)
- Secrets encrypted with AES-256-GCM (or OS keychain on macOS/Linux)
- No telemetry, analytics, or data sharing
- Full audit log of all tool executions

---

## Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                          Channels                              │
│  ┌──────┐  ┌──────┐   ┌─────────────┐  ┌─────────────┐       │
│  │ REPL │  │ HTTP │   │WASM Channels│  │ Web Gateway │       │
│  └──┬───┘  └──┬───┘   │  Telegram   │  │ (SSE + WS)  │       │
│     │         │        │  Slack      │  └──────┬──────┘       │
│     │         │        │  Discord    │         │              │
│     │         │        └──────┬──────┘         │              │
│     └─────────┴──────────────┴─────────────────┘              │
│                              │                                 │
│                    ┌─────────▼─────────┐                       │
│                    │    Agent Loop     │  Intent routing       │
│                    └────┬──────────┬───┘                       │
│                         │          │                           │
│              ┌──────────▼────┐  ┌──▼───────────────┐          │
│              │  Scheduler    │  │ Routines Engine  │          │
│              │(parallel jobs)│  │(cron, event, wh) │          │
│              └──────┬────────┘  └────────┬─────────┘          │
│                     │                    │                     │
│       ┌─────────────┼────────────────────┘                     │
│       │             │                                          │
│   ┌───▼─────┐  ┌────▼────────────────┐                        │
│   │ Local   │  │    Orchestrator     │                        │
│   │Workers  │  │  ┌───────────────┐  │                        │
│   │(in-proc)│  │  │ Docker Sandbox│  │                        │
│   └───┬─────┘  │  │   Containers  │  │                        │
│       │        │  └───────────────┘  │                        │
│       │        └─────────┬───────────┘                        │
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
| **Subagent Executor** | Parallel sub-agent orchestration with tool filtering and sandboxing |
| **Orchestrator** | Container lifecycle, LLM proxying, per-job auth |
| **Web Gateway** | Browser UI with chat, memory, jobs, logs, costs, extensions, routines |
| **Canvas Gateway** | Rich interactive UI canvas for agent-generated content |
| **Routines Engine** | Scheduled (cron) and reactive (event, webhook) background tasks |
| **Workspace** | Persistent memory with hybrid full-text + vector search |
| **Safety Layer** | Prompt injection defense, leak detection, key rotation, content sanitization |
| **LLM Runtime Manager** | Live provider switching, routing policy, cost-aware model selection |
| **Cost Guard** | Token usage tracking and configurable spending limits |
| **ClawHub** | Extension marketplace — discover, install, and manage tools |
| **Skill Registry** | Structured multi-step procedures with hot-reload and proactive discovery |
| **Hardware Bridge** | Sensor access (camera, mic, screen) with per-request approval |
| **Health Monitor** | Channel health checking and automatic recovery |

### Feature Flags

The default build profile is `light` — a minimal, fast-compiling configuration suitable for most use cases. Use `--features full` for the complete feature set.

See [docs/BUILD_PROFILES.md](docs/BUILD_PROFILES.md) for the full build profiles guide.

| Feature | Description | Included by default |
|---------|-------------|---------------------|
| `light` | Core agent + databases + HTML/doc extraction | yes (default profile) |
| `full` | Everything: light + web gateway + REPL + tunnel + Docker | no — opt in |
| `desktop` | Tauri/Scrappy embedding (libSQL, minimal footprint) | no — opt in |
| `postgres` | PostgreSQL + pgvector + TLS support | yes (via light) |
| `libsql` | Embedded libSQL/Turso support | yes (via light) |
| `html-to-markdown` | Web page to markdown conversion | yes (via light) |
| `document-extraction` | PDF/DOCX/PPTX/XLSX text extraction | yes (via light) |
| `repl` | Interactive terminal REPL + boot screen | yes (via full) |
| `web-gateway` | Browser UI with SSE/WebSocket streaming | yes (via full) |
| `tunnel` | Managed tunnels (ngrok, Cloudflare, Tailscale) for public webhooks | yes (via full) |
| `docker-sandbox` | Isolated container execution for untrusted code | yes (via full) |
| `voice` | Voice wake word detection (cpal audio capture) | no — opt in |
| `bundled-wasm` | Embed all WASM extensions in binary for air-gapped deploys (+6-13 MB) | no — opt in |

---

## WASM Extension Deployment

ThinClaw ships 14 WASM extensions (10 tools + 4 channels). Two deployment strategies are available:

**Option A: Download from GitHub Releases (default)**

The standard binary downloads extensions from GitHub Releases on first install. The CI pipeline (`release.yml`) automatically builds all WASM extensions, creates `.tar.gz` bundles with SHA256 checksums, and publishes them alongside the binary.

```bash
# Standard build — small binary, extensions downloaded on demand
cargo build --release

# Install an extension at runtime (downloads from GitHub Releases)
thinclaw extension install telegram
```

**Option B: Embedded WASM (air-gapped / zero-network)**

```bash
# Air-gapped build — all extensions embedded (+6-13 MB binary size)
cargo build --release --features bundled-wasm

# Or use the convenience script:
./scripts/build-all.sh --bundled
```

When `bundled-wasm` is active, `thinclaw extension install <name>` extracts the extension from the binary — zero network dependency.

**Install priority chain:**
1. Bundled WASM (if `--features bundled-wasm` was used at compile time)
2. Download from GitHub Releases
3. Build from local source tree (developer builds only)

---

## Development

```bash
# Format code
cargo fmt

# Lint
cargo clippy --all --benches --tests --examples --all-features

# Run tests
createdb thinclaw_test
cargo test

# Run specific test
cargo test test_name
```

- **Deployment & Remote Setup**: See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)
- **Telegram channel**: See [docs/TELEGRAM_SETUP.md](docs/TELEGRAM_SETUP.md)
- **Gmail OAuth setup**: See [docs/GMAIL_SETUP.md](docs/GMAIL_SETUP.md)
- **Extension development**: See [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md)
- **Channel architecture**: See [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md)
- **Changing channel sources**: Run `./scripts/build-all.sh` to rebuild all WASM channels and the main binary
- **Air-gapped builds**: Use `cargo build --release --features bundled-wasm` to embed all WASM extensions in the binary

---

## Credits & Heritage

ThinClaw is an evolved Rust reimplementation of [IronClaw](https://github.com/nearai/ironclaw) (NEAR AI) and draws inspiration from [OpenClaw](https://github.com/openclaw/openclaw). We are grateful for the foundation these projects provided.

ThinClaw has grown significantly beyond its origins:

| Dimension | ThinClaw additions |
|-----------|-------------------|
| **Language** | Rust — native performance, memory safety, single binary |
| **Security** | WASM sandbox, 3-layer shell sandbox, SSRF guards, workspace sandbox modes |
| **Cost control** | Cost Guard with hard limits, smart routing, OpenRouter pricing sync |
| **Skills** | Hot-reload, workspace-level overrides, proactive discovery, attenuation |
| **Providers** | OpenRouter, Tinfoil, Bedrock (native), Gemini, Perplexity, NVIDIA, MiniMax |
| **Storage** | libSQL + PostgreSQL + Turso cloud sync |
| **Routing** | RoutingPolicy engine, drag-and-drop provider pools, WebUI Provider Vault |
| **Tests** | Integration test suite for all major subsystems |

See [FEATURE_PARITY.md](FEATURE_PARITY.md) for the complete OpenClaw feature tracking matrix.

---

## License

Copyright © 2026 **RNT56** (ThinClaw)  
Copyright © 2026 **NEAR AI** (IronClaw — original codebase, used as starting point)

ThinClaw began as a reimplementation of IronClaw by NEAR AI.  
The original IronClaw code is used under the terms of its MIT / Apache-2.0 dual license.

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
