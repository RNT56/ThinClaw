# IronClaw Deployment Paths — Current State

> Comprehensive reference of every way IronClaw can be deployed today,
> what each path offers, and how it works under the hood.
>
> **Last updated:** See git log for latest revision date

---

## Architecture Overview

IronClaw is a **dual-mode** Rust agent system. It can run as:

1. **Embedded Library** — inside the Scrappy desktop app (Tauri), using in-process IPC
2. **Standalone Binary** — headless server process with its own HTTP/WS gateway

Both modes share the same core: `AppBuilder → AppComponents → Agent → ChannelManager`.
The difference is *how messages enter and leave* the agent loop.

```
┌──────────────────────────────────────────────────┐
│                  IronClaw Core                    │
│  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │ AppBuilder│→│ AgentDeps │→│ Agent.run()    │  │
│  │ (5 phases)│  │ (LLM,    │  │ or             │  │
│  │           │  │  Tools,  │  │ handle_message │  │
│  │           │  │  Safety) │  │ _external()    │  │
│  └──────────┘  └──────────┘  └───────────────┘  │
│                        ↕                          │
│  ┌───────────────────────────────────────────┐   │
│  │           ChannelManager                   │   │
│  │  ┌─────┐ ┌────────┐ ┌─────┐ ┌────────┐   │   │
│  │  │REPL │ │Gateway │ │Tele │ │ Tauri  │   │   │
│  │  │     │ │(HTTP/  │ │gram │ │Channel │   │   │
│  │  │     │ │WS/SSE) │ │     │ │(IPC)   │   │   │
│  │  └─────┘ └────────┘ └─────┘ └────────┘   │   │
│  └───────────────────────────────────────────┘   │
└──────────────────────────────────────────────────┘
```

---

## Repository Structure

IronClaw lives inside the Scrappy monorepo as a subfolder — **not** as a separate git repo or submodule.

```
scrappy/                       ← single git repo
├── frontend/                  ← Scrappy React/TypeScript UI
├── backend/src/               ← Scrappy Tauri Rust backend
│   └── openclaw/              ← Bridge code (commands, proxy, bridge)
├── ironclaw/                  ← IronClaw agent engine (Rust library + binary)
│   ├── src/                   ← Core agent code
│   ├── Cargo.toml             ← Standalone crate (also used as lib by backend)
│   ├── Dockerfile             ← Multi-stage Docker build
│   └── deploy/                ← Remote deployment bundle
│       ├── docker-compose.yml
│       ├── .env.template
│       └── setup.sh           ← Automated Linux setup script
└── Cargo.toml                 ← Workspace root (members: backend, ironclaw)
```

The `backend/Cargo.toml` depends on `ironclaw` as a path dependency (`ironclaw = { path = "../ironclaw" }`). There is no git submodule involved.

---

## Deployment Path 1: Embedded in Scrappy (Desktop App)

### Status: ✅ Fully Implemented — Primary Path

### How It Works

| Aspect | Detail |
|---|---|
| **Entry point** | `ironclaw_bridge.rs → IronClawState::start()` |
| **Channel** | `TauriChannel` — messages flow via Tauri's `invoke` / `emit` IPC |
| **Agent loop** | No `agent.run()`. Each message triggers `agent.handle_message_external()` individually |
| **Config source** | `identity.json` (Scrappy's app data dir) + macOS Keychain for secrets |
| **LLM routing** | Can use local inference (MLX sidecar) or cloud providers. Configured via Settings UI |
| **Database** | LibSQL (local embedded SQLite superset), auto-created |
| **Gateway server** | ❌ None — no HTTP port is opened |

### What This Offers
- Full agent capabilities (tools, memory, sessions, routines, skills, extensions)
- macOS Keychain-backed secrets (API keys never touch disk)
- Hardware bridge for camera/mic/screen capture
- User-facing UI for all settings
- OS permissions management (Accessibility, Screen Recording)
- Identity ritual / bootstrap flow

### Limitations
- **Local only** — no network-accessible API
- Cannot be accessed by Telegram, Slack, Discord, or external systems
- Agent is tied to the desktop app lifecycle (app closed = agent stops)

### Key Files
- `backend/src/openclaw/ironclaw_bridge.rs` — lifecycle management
- `backend/src/openclaw/ironclaw_channel.rs` — `TauriChannel` impl
- `backend/src/openclaw/commands/gateway.rs` — start/stop Tauri commands
- `frontend/src/components/settings/GatewayTab.tsx` — settings UI

---

## Deployment Path 2: Remote via Scrappy UI (Deploy New Agent)

### Status: ✅ Fully Implemented — New

### How It Works

The Scrappy Desktop app can automatically deploy IronClaw to a Linux server and connect to it. This is the **recommended path for remote deployment**.

| Aspect | Detail |
|---|---|
| **Trigger** | Scrappy UI → Settings → Gateway → "Deploy New Remote Agent" |
| **Backend command** | `openclaw_deploy_remote(ip, user)` in `backend/src/openclaw/deploy.rs` |
| **Transport** | SSH + SCP from the user's machine to the target Linux server |
| **What runs remotely** | `docker compose up -d` using `ironclaw/deploy/docker-compose.yml` |
| **Target OS** | **Linux only** (Ubuntu/Debian primary, RHEL/CentOS secondary) |
| **Auth** | Auto-generated 64-char hex token, returned to UI |

### Deploy Flow (Step by Step)

1. User enters SSH host IP and user (e.g., `root@203.0.113.50`) in the RemoteDeployWizard UI.
2. Backend finds the `ironclaw/deploy/` bundle locally.
3. `scp -r` copies the deploy folder to `/tmp/ironclaw-deploy` on the remote server.
4. `ssh` runs `setup.sh --token <auto-generated-token>` on the remote server, which:
   - Installs Docker if missing (via `apt-get` or `yum`).
   - Creates `.env` from `.env.template`, inserts the generated token.
   - Runs `docker compose up -d --build`.
5. Backend polls `http://<ip>:18789/api/health` for up to 30 seconds.
6. Emits structured `deploy-status` JSON event `{ status, url, token }` to the frontend.
7. Frontend shows result card — user clicks "Save & Connect".
8. Profile is created, gateway settings updated, `openclaw_start_gateway` activates the `RemoteGatewayProxy`.

### What This Offers
- One-click deployment from the desktop app
- No manual SSH or terminal required
- Auto-generates secure auth token
- Automatic Docker installation on target
- Immediate connection after deploy

### Limitations
- **Linux targets only** (setup.sh uses apt/yum)
- Requires SSH key-based access from the user's machine
- Port 18789 must be open on the target server's firewall
- No fallback bundle if target server has no internet for Docker image pulls

### Key Files
- `backend/src/openclaw/deploy.rs` — SSH+SCP deploy command
- `ironclaw/deploy/setup.sh` — remote setup script
- `ironclaw/deploy/docker-compose.yml` — Docker Compose stack
- `ironclaw/deploy/.env.template` — environment template
- `frontend/src/components/openclaw/RemoteDeployWizard.tsx` — wizard UI

---

## Deployment Path 3: Remote via Scrappy UI (Connect Existing)

### Status: ✅ Fully Implemented — New

### How It Works

If IronClaw is already running on a remote machine (set up manually, or by a previous deploy), the Scrappy app can connect to it.

| Aspect | Detail |
|---|---|
| **Trigger** | Scrappy UI → Settings → Gateway → "Connect Existing" tab |
| **Input** | Gateway URL (`http://<ip>:18789`) + Auth Token |
| **Connection test** | `openclaw_test_connection(url, token)` → real HTTP GET to `/api/health` |
| **Activation** | `openclaw_save_gateway_settings` + `openclaw_start_gateway` |

### Connect Flow

1. User enters URL and token in the "Connect Existing" tab.
2. Frontend calls `openclaw_test_connection()` which creates a temporary `RemoteGatewayProxy` and calls `health_check()`.
3. On success, creates an agent profile and persists it via `openclaw_save_gateway_settings`.
4. Calls `openclaw_start_gateway`, which detects `gateway_mode == "remote"` and:
   - Creates the real `RemoteGatewayProxy`.
   - Runs `health_check()` again.
   - Starts the SSE subscription (auto-reconnecting background task).
   - Activates the proxy in `IronClawState`.
5. All UI commands now transparently route through the proxy to the remote agent.

### What This Offers
- Connect to **any** running IronClaw instance, regardless of how it was set up
- Works for macOS, Windows, Linux, or cloud targets
- Real health check before connecting
- Profile-based management (switch between local and multiple remote agents)

### Key Files
- `backend/src/openclaw/commands/gateway.rs` — `openclaw_test_connection`, `openclaw_start_gateway`
- `backend/src/openclaw/remote_proxy.rs` — HTTP/SSE proxy client
- `frontend/src/components/openclaw/RemoteDeployWizard.tsx` — connect UI

---

## Deployment Path 4: Manual Docker Setup (Any OS, No Scrappy UI Needed)

### Status: ✅ Fully Supported

### How It Works

Set up IronClaw standalone using Docker, with no Scrappy app involved at all.

```bash
# On the target machine (Linux, macOS with Docker Desktop, or Windows with Docker Desktop):
cd ironclaw/deploy/
cp .env.template .env
nano .env   # Set GATEWAY_AUTH_TOKEN and at least one LLM API key
docker compose up -d --build
```

### docker-compose.yml Stack

The `ironclaw/deploy/docker-compose.yml` provides:
- **IronClaw container** — builds from `ironclaw/Dockerfile`, exposes port 18789
- **Optional PostgreSQL** — pgvector-enabled, activated with `--profile postgres`
- Named volumes for data persistence
- Health check on `/api/health`
- Auto-restart policy (`unless-stopped`)

### Connecting After Manual Setup

**Option A: Use Scrappy Desktop App**
Use "Connect Existing" in Gateway Settings (Deployment Path 3 above).

**Option B: Use the built-in Web UI**
Navigate to `http://<ip>:18789` in a browser — IronClaw includes an embedded web chat.

**Option C: Use Telegram / Discord / CLI**
Configure the channels in `.env` and the agent is reachable via those channels directly, no Scrappy needed.

### What This Offers
- Works on **any Docker-capable OS** (Linux, macOS, Windows)
- Complete isolation from host system
- Easy updates (`docker compose pull && docker compose up -d`)
- Optional PostgreSQL + pgvector for embeddings

### Key Files
- `ironclaw/deploy/docker-compose.yml` — Compose stack
- `ironclaw/deploy/.env.template` — config template
- `ironclaw/Dockerfile` — multi-stage Rust build

---

## Deployment Path 5: Direct Binary on Server (No Docker)

### Status: ✅ Fully Supported

### How It Works

The simplest headless deployment — compile and run the Rust binary directly:

```bash
# On the server
git clone <repo>
cd ironclaw/
cp .env.example .env
# Edit .env with your configuration

# Build from source
cargo build --release --features libsql

# Run
./target/release/ironclaw run
```

### Minimal .env for Remote Access

```env
# LLM
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-...

# Gateway (network accessible)
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=18789
GATEWAY_AUTH_TOKEN=your-secret-token

# Database (local SQLite via LibSQL)
DATABASE_BACKEND=libsql
LIBSQL_PATH=./data/ironclaw.db

# Optional: Add Telegram
TELEGRAM_BOT_TOKEN=123:ABC...
TELEGRAM_OWNER_ID=12345

# Agent
AGENT_NAME=my-remote-agent
```

### What This Offers
- No Docker required
- Full control over the binary
- All channels available
- Multi-channel simultaneous operation (REPL + Gateway + Telegram + Discord)

### Key Files
- `ironclaw/src/main.rs` — binary entry point
- `ironclaw/.env.example` — full env reference

---



## Channels Available in Standalone Mode

| Channel | Env Config | Description |
|---|---|---|
| **REPL** | `CLI_ENABLED=true` (default) | Terminal-based interactive chat |
| **Gateway** | `GATEWAY_HOST`, `GATEWAY_PORT`, `GATEWAY_AUTH_TOKEN` | Full HTTP API + WebSocket + SSE + embedded web UI |
| **Telegram** | `TELEGRAM_BOT_TOKEN`, `TELEGRAM_OWNER_ID` | Long-polling Telegram bot |
| **Discord** | `DISCORD_BOT_TOKEN`, `DISCORD_GUILD_ID` | Discord Gateway WebSocket bot |
| **Signal** | `SIGNAL_HTTP_URL`, `SIGNAL_ALLOW_FROM` | Via signal-cli REST API |
| **Nostr** | `NOSTR_PRIVATE_KEY`, `NOSTR_RELAYS` | Nostr protocol messaging |
| **Gmail** | `GMAIL_PROJECT_ID`, `GMAIL_SUBSCRIPTION_ID` | Google Pub/Sub pull |
| **iMessage** | `IMESSAGE_ALLOW_FROM` | macOS only, chat.db polling |
| **HTTP** | `HTTP_HOST`, `HTTP_PORT` | Simple webhook endpoint |
| **WASM** | `WASM_CHANNELS_DIR` | Plugin-based channels (WhatsApp, custom) |

---

## Gateway HTTP API Endpoints

The standalone gateway server (`channels/web/server.rs`) exposes:

| Endpoint | Method | Description |
|---|---|---|
| `/api/health` | GET | Health check (public, no auth) |
| `/api/chat/send` | POST | Send message |
| `/api/chat/events` | GET | SSE event stream |
| `/api/chat/ws` | GET | WebSocket upgrade |
| `/api/chat/threads` | GET | List sessions |
| `/api/chat/threads/delete` | POST | Delete a session |
| `/api/chat/threads/reset` | POST | Reset (clear) a session |
| `/api/chat/history` | GET | Get chat history |
| `/api/chat/export` | GET | Export session transcript |
| `/api/chat/approval` | POST | Resolve tool approval |
| `/api/memory/read` | GET | Read workspace file |
| `/api/memory/write` | POST | Write workspace file |
| `/api/memory/delete` | POST | Delete workspace file |
| `/api/memory/list` | GET | List workspace files |
| `/api/memory/search` | POST | Search workspace |
| `/api/gateway/status` | GET | Full status + diagnostics |
| `/api/settings/{key}` | GET/PUT | Read/write settings |
| `/api/routines` | GET | List routines |
| `/api/routines/{id}/trigger` | POST | Trigger a routine |
| `/api/skills` | GET | List skills |
| `/api/extensions` | GET | List extensions |
| `/v1/chat/completions` | POST | OpenAI-compatible endpoint |

---

## Comparison Matrix

| Feature | Scrappy Desktop | Scrappy → Remote | Manual Docker | Direct Binary |
|---|:---:|:---:|:---:|:---:|
| **Network API** | ❌ | ✅ Gateway | ✅ Gateway | ✅ Gateway |
| **Telegram/Discord** | ❌ | ✅ | ✅ | ✅ |
| **REPL CLI** | ❌ (Tauri IPC) | ❌ (Docker) | ❌ (Docker) | ✅ (SSH) |
| **Settings UI** | ✅ Scrappy | ✅ Scrappy (remote) | ✅ Web UI | ✅ Web UI |
| **Secrets** | macOS Keychain | Proxy → remote .env | .env / Docker | .env / DB |
| **Database** | LibSQL (auto) | LibSQL / PG | LibSQL / PG | LibSQL / PG |
| **macOS Features** | ✅ | ❌ | ❌ | ✅ (if on Mac) |
| **Auto-start** | Scrappy launch | docker restart | docker restart | systemd |
| **OS for target** | macOS | Linux | Any (Docker) | Any (Rust) |
| **Difficulty** | Easiest | Easy (1-click) | Easy | Medium |

---

## Configuration Reference

### Environment Variables (Standalone / Docker)

See `ironclaw/deploy/.env.template` for the full list. Key categories:

- **LLM**: `LLM_BACKEND`, `LLM_MODEL`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc.
- **Gateway**: `GATEWAY_HOST`, `GATEWAY_PORT`, `GATEWAY_AUTH_TOKEN`
- **Database**: `DATABASE_BACKEND`, `DATABASE_URL`, `LIBSQL_PATH`
- **Channels**: `TELEGRAM_BOT_TOKEN`, `DISCORD_BOT_TOKEN`, `SIGNAL_HTTP_URL`, etc.
- **Agent**: `AGENT_NAME`, `AGENT_AUTO_APPROVE_TOOLS`
- **Sandbox**: `SANDBOX_ENABLED`
- **Heartbeat**: `HEARTBEAT_ENABLED`, `HEARTBEAT_INTERVAL_SECS`
- **Embeddings**: `EMBEDDING_ENABLED`

### Scrappy Desktop Config

All settings stored in `identity.json` inside the app data directory:
```
~/Library/Application Support/OpenClaw/identity.json
```

Gateway mode fields:
- `gateway_mode`: `"local"` or `"remote"`
- `remote_url`: `"http://192.168.1.50:18789"`
- `remote_token`: `"<64-char-hex>"`
- `profiles`: Array of `{ id, name, mode, url, token }`

Secrets (API keys) stored in macOS Keychain under service name `openclaw`.
