# ThinClaw Deployment Guide

This guide covers every way to deploy ThinClaw as a standalone agent and connect it to the **Scrappy** desktop app. Whether you're setting up a dedicated Mac Mini, a Linux VPS, or a Docker container, this document has you covered.

---

## Table of Contents

- [Prerequisites](#prerequisites)
- [Deployment Paths](#deployment-paths)
  - [Path 1: Direct Binary (Mac Mini / macOS / Linux)](#path-1-direct-binary-recommended-for-mac-mini)
  - [Path 2: Docker Compose (Any OS)](#path-2-docker-compose-any-os)
  - [Path 3: Automated Deploy from Scrappy](#path-3-automated-deploy-from-scrappy-linux-targets)
  - [Path 4: Automated Linux Server Setup](#path-4-automated-linux-server-setup)
- [Development Docker Compose](#development-docker-compose)
- [Docker Images Reference](#docker-images-reference)
- [WASM Extension Deployment](#wasm-extension-deployment)
- [Connecting Scrappy to ThinClaw](#connecting-scrappy-to-thinclaw)
- [Securing the Connection (Tailscale)](#securing-the-connection-tailscale)
- [Auto-Start on Boot](#auto-start-on-boot)
  - [macOS: launchd](#macos-launchd)
  - [Linux: systemd](#linux-systemd)
- [macOS-Specific Features](#macos-specific-features)
- [Environment Reference](#environment-reference)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

### Pre-Built Binary (download from Releases)

If you install ThinClaw via the [shell/PowerShell installer](https://github.com/RNT56/ThinClaw/releases), **no prerequisites are needed** — the binary is self-contained. WASM extensions are downloaded from GitHub Releases on first install.

### Compile from Source

To build ThinClaw from source, you need the following tools:

| Prerequisite | Purpose | Install Command |
|---|---|---|
| **Xcode CLI Tools** (macOS) | C compiler, linker | `xcode-select --install` |
| **build-essential + libssl-dev** (Linux) | C compiler, OpenSSL headers | `sudo apt install build-essential pkg-config libssl-dev` |
| **Rust 1.92+** | Rust compiler and Cargo | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **wasm32-wasip2 target** | WASM compilation target | `rustup target add wasm32-wasip2` |
| **wasm-tools** | WASM component model conversion | `cargo install wasm-tools --locked` |
| **cargo-component** | Build WASM extensions from source | `cargo install cargo-component --locked` |
| **Git** | Clone the repository | Pre-installed on macOS; `apt install git` on Linux |

**Optional:**

| Prerequisite | Purpose | Install Command |
|---|---|---|
| **Docker** | Docker sandbox feature, Docker Compose deployment | `brew install docker` (macOS) or [docs.docker.com](https://docs.docker.com/engine/install/) |
| **PostgreSQL 15+ + pgvector** | Production database (alternative to libSQL) | `brew install postgresql@15` (macOS) or `apt install postgresql` |

### One-Click Setup Scripts

Instead of installing prerequisites manually, use the provided setup scripts:

```bash
# macOS — installs everything and builds ThinClaw:
./scripts/mac-deploy.sh

# macOS — prerequisites only (no build):
./scripts/mac-deploy.sh --install-only

# Developer setup (any OS) — installs WASM toolchain, runs tests:
./scripts/dev-setup.sh
```

---

## Architecture Overview

ThinClaw is a **dual-mode** Rust agent. It can run as:

1. **Embedded Library** — inside the Scrappy desktop app (Tauri), using in-process IPC
2. **Standalone Binary** — headless server with its own HTTP/WebSocket/SSE gateway

Both modes share the same core. The difference is how messages enter and leave the agent.

**Standalone Architecture:**

```
┌─────────────────────┐         HTTP/SSE          ┌────────────────────────┐
│  Your MacBook/iMac  │ ◄═══════════════════════► │  Dedicated Server      │
│                     │         port 18789         │  (Mac Mini / VPS)      │
│  Scrappy Desktop    │                            │                        │
│  "Remote Mode"      │                            │  ThinClaw Agent        │
│                     │                            │  ├─ LLM inference      │
│  Controls the agent │                            │  ├─ Tool execution     │
│  via Gateway proxy  │                            │  ├─ Persistent memory  │
│                     │                            │  └─ Channels (Telegram,│
│                     │                            │     Discord, iMessage) │
└─────────────────────┘                            └────────────────────────┘
```

The Scrappy app acts as the **control plane** — it sends messages and displays responses. The server runs the **heavy lifting** (agent loop, LLM calls, tool execution, persistent memory).

---

## Deployment Paths

### Path 1: Direct Binary (Recommended for Mac Mini)

Best for: **macOS (Mac Mini / Mac Studio)** or any machine where you want native performance and access to OS features (Keychain, iMessage, Metal GPU).

#### One-Click Deploy (macOS)

```bash
# From a fresh Mac Mini — installs everything, builds, and launches:
curl -fsSL https://raw.githubusercontent.com/RNT56/ThinClaw/main/scripts/mac-deploy.sh | bash

# Air-gapped variant (embeds all WASM extensions in binary):
curl -fsSL https://raw.githubusercontent.com/RNT56/ThinClaw/main/scripts/mac-deploy.sh | bash -s -- --bundled
```

#### Manual Setup

```bash
# macOS
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
rustup target add wasm32-wasip2
cargo install wasm-tools --locked
cargo install cargo-component --locked

# Linux (Ubuntu/Debian)
sudo apt install build-essential pkg-config libssl-dev git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
rustup target add wasm32-wasip2
cargo install wasm-tools --locked
cargo install cargo-component --locked
```

#### Build

```bash
# Clone the repo
git clone https://github.com/RNT56/ThinClaw.git
cd ThinClaw

# Build with LibSQL (recommended for single-server setups)
cargo build --release --features libsql

# Or build with both database backends
cargo build --release --features "libsql,postgres"

# Air-gapped build (all WASM extensions embedded in binary):
cargo build --release --features "libsql,bundled-wasm"
```

The binary will be at `./target/release/thinclaw`.

#### First Run — Onboarding Wizard

On first launch, ThinClaw auto-detects that no database is configured and launches the interactive **9-step setup wizard**:

```bash
./target/release/thinclaw
```

| Step | What It Configures |
|------|------|
| 1. Database | LibSQL local file (`~/.thinclaw/thinclaw.db`) — press Enter for defaults |
| 2. Security | Master key for encrypting secrets — choose **macOS Keychain** on Mac |
| 3. Inference Provider | LLM backend: Anthropic, OpenAI, Ollama, OpenRouter, or any OpenAI-compatible |
| 4. Model Selection | Pick a model from the provider's catalog |
| 5. Embeddings | Semantic memory search (recommended: yes) |
| 6. Channels | **Enable Gateway** (required for Scrappy) + optional Telegram/Discord/iMessage |
| 7. Extensions | Install WASM tool plugins from registry |
| 8. Docker Sandbox | Optional isolated execution (skip for simple setups) |
| 9. Heartbeat | Optional periodic background tasks |

**⚠️ Critical: Gateway Configuration (Step 6)**

You **must** enable the Gateway channel for Scrappy to connect. When prompted:

| Setting | Value | Notes |
|---------|-------|-------|
| **Gateway Host** | `0.0.0.0` | Listen on all network interfaces |
| **Gateway Port** | `18789` | Default port (or choose your own) |
| **Auth Token** | Generate one | Run `openssl rand -hex 32` — save this token! |

#### First Run — Manual Configuration (Alternative)

If you prefer to skip the wizard, create `~/.thinclaw/.env` manually:

```env
# LLM Provider (choose one)
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-...

# Or use OpenRouter for multi-provider access:
# LLM_BACKEND=openai_compatible
# LLM_BASE_URL=https://openrouter.ai/api/v1
# LLM_API_KEY=sk-or-...
# LLM_MODEL=anthropic/claude-sonnet-4-5

# Gateway (REQUIRED for Scrappy connection)
GATEWAY_ENABLED=true
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=18789
GATEWAY_AUTH_TOKEN=your-64-char-secret-token-here

# Database
DATABASE_BACKEND=libsql
LIBSQL_PATH=/Users/yourname/.thinclaw/thinclaw.db

# Agent Identity
AGENT_NAME=mac-mini-agent

# Optional: iMessage channel (macOS only!)
# IMESSAGE_ALLOW_FROM=+1234567890,+0987654321

# Optional: Telegram
# TELEGRAM_BOT_TOKEN=123456:ABC-DEF...
# TELEGRAM_OWNER_ID=12345678

# Optional: Embeddings
EMBEDDING_ENABLED=true
EMBEDDING_PROVIDER=openai
OPENAI_API_KEY=sk-...
```

Then run with:

```bash
./target/release/thinclaw run --no-onboard
```

#### Verify

ThinClaw prints a boot screen on startup:

```
╭──────────────────────────────────────╮
│  🦀 ThinClaw v0.12.0                │
│  Agent: mac-mini-agent               │
│  LLM: anthropic (claude-sonnet-4-5)  │
│  Database: libsql ✓                  │
│  Tools: 12                           │
│  Web UI: http://0.0.0.0:18789/       │
│  Channels: gateway, repl             │
╰──────────────────────────────────────╯
```

Test the gateway:

```bash
# From the server itself
curl http://localhost:18789/api/health

# From another machine on the network
curl http://<server-ip>:18789/api/health
```

A successful response returns `{"status":"ok", ...}`.

---

### Path 2: Docker Compose (Any OS)

Best for: **Linux VPS / cloud servers**, or any machine with Docker Desktop (macOS/Windows).

```bash
cd deploy/
cp .env.template .env

# Edit .env: set GATEWAY_AUTH_TOKEN and at least one LLM key
nano .env

# Start
docker compose up -d --build
```

The container exposes port `18789`. Connect Scrappy to `http://<server-ip>:18789`.

For PostgreSQL with pgvector (production):

```bash
docker compose --profile postgres up -d --build
```

See `deploy/.env.template` for all available configuration options.

---

### Path 3: Automated Deploy from Scrappy (Linux Targets)

The Scrappy desktop app can deploy ThinClaw to a Linux server in one click:

1. Go to **Settings → Gateway** in the Scrappy App.
2. Click **Deploy New Remote Agent**.
3. Enter your server's **SSH Host IP** and **SSH User** (e.g., `root`).
4. Click **Deploy via SSH**.

Scrappy will SCP the deploy bundle, install Docker, generate an auth token, and start the agent. After completion, click **Save & Connect**.

**Requirements:** SSH key-based access to a Linux server. Port 18789 must be reachable.

---

### Path 4: Automated Linux Server Setup

Best for: **Fresh Linux VPS / cloud servers** where you want Docker + firewall + optional Tailscale set up automatically.

The `deploy/setup.sh` script bootstraps a Linux server from scratch:

```bash
# Minimal — installs Docker, UFW firewall, Fail2ban, starts ThinClaw
sudo bash deploy/setup.sh --token <gateway_auth_token>

# Full production — adds Tailscale VPN + systemd auto-start
sudo bash deploy/setup.sh --token <token> --tailscale <ts-authkey> --systemd
```

**What it installs and configures (6 steps):**

| Step | What | Details |
|------|------|---------|
| 1 | Docker Engine + Compose V2 | Adds official Docker repo, installs `docker-ce`, enables on boot |
| 2 | UFW Firewall | Denies incoming by default, allows SSH + port 18789 |
| 3 | Fail2ban | SSH brute-force protection — 3 retries, 1 hour ban |
| 4 | Tailscale VPN *(optional)* | Installs Tailscale, joins network with auth key, restricts gateway to Tailscale-only |
| 5 | ThinClaw Container | Copies `.env.template` → `.env`, injects token, runs `docker compose up -d --build` |
| 6 | systemd service *(optional)* | Creates a systemd unit so `docker compose up` runs on boot |

**Supported package managers:** apt (Ubuntu/Debian), yum/dnf (RHEL/CentOS/Fedora).

---

## Development Docker Compose

The root `docker-compose.yml` provides a **local PostgreSQL** instance for development:

```bash
# Start a pgvector-enabled Postgres for development
docker compose up -d
```

This starts `pgvector/pgvector:pg16` on port 5432 with dev-only credentials (`ironclaw`/`ironclaw`). Use this when developing with the `postgres` feature flag instead of libSQL.

> **Note:** This is for local development only. For production, use the `deploy/docker-compose.yml` stack or Path 4's automated setup.

---

## Docker Images Reference

ThinClaw ships three Dockerfiles for different purposes:

| Image | Dockerfile | Purpose | Key Contents |
|-------|-----------|---------|-------------|
| `thinclaw` | `Dockerfile` | Cloud/remote headless agent | Minimal runtime — thinclaw binary + ca-certs + curl |
| `thinclaw-worker` | `Dockerfile.worker` | Sandboxed job execution | Full dev toolchain — Rust, Node.js, Python, Git, GitHub CLI, Claude Code |
| `thinclaw-sandbox` | `docker/sandbox.Dockerfile` | WASM compilation sandbox | Rust + WASM targets + wasm-tools (no dev tools) |

Build commands:

```bash
# Cloud agent
docker build --platform linux/amd64 -t thinclaw:latest .

# Worker (used by the orchestrator for Docker sandbox jobs)
docker build -f Dockerfile.worker -t thinclaw-worker .

# WASM sandbox (lightweight)
docker build -f docker/sandbox.Dockerfile -t thinclaw-sandbox .
```

All images run as non-root users and use Rust 1.92 / Debian bookworm.

---

## WASM Extension Deployment

ThinClaw ships 14 WASM extensions (10 tools + 4 channels). Two strategies are available depending on your deployment environment:

### Option A: Download from GitHub Releases (Default)

The standard binary downloads extensions from GitHub Releases on first install. The CI pipeline (`release.yml`) automatically builds all WASM extensions, creates `.tar.gz` bundles with SHA256 checksums, and publishes them as release assets.

```bash
# Standard build — small binary, extensions downloaded on demand
cargo build --release

# Install an extension at runtime (downloads from GitHub Releases)
thinclaw extension install telegram
```

This is the recommended approach for machines with internet access.

### Option B: Embedded WASM (Air-Gapped / Zero-Network)

For headless deployments on machines without reliable internet (e.g., Mac Mini behind NAT), compile with the `bundled-wasm` feature to embed all WASM extensions directly into the binary:

```bash
# Air-gapped build — all extensions embedded (+6-13 MB binary size)
cargo build --release --features bundled-wasm

# Or use the convenience script:
./scripts/build-all.sh --bundled

# Or use the 1-click Mac deploy:
./scripts/mac-deploy.sh --bundled
```

When `bundled-wasm` is active, `thinclaw extension install <name>` extracts the extension from the binary instead of downloading — zero network dependency.

**Install priority chain:**
1. Bundled WASM (if `--features bundled-wasm` was used at compile time)
2. Download from GitHub Releases (if `artifacts.url` is set in the manifest)
3. Build from local source tree (developer builds only)

---

## Connecting Scrappy to ThinClaw

Once ThinClaw is running (via any path above), connect your Scrappy desktop app:

1. Open the Scrappy app on your main machine.
2. Go to **Settings → Gateway**.
3. Click **Add New Agent Profile** → select **"Connect Existing"**.
4. Enter:
   - **Gateway URL:** `http://<server-ip>:18789`
     (e.g., `http://192.168.1.50:18789` or `http://100.64.0.2:18789` for Tailscale)
   - **Auth Token:** The value you set as `GATEWAY_AUTH_TOKEN`.
5. Click **Test & Save**.

Scrappy verifies the connection via `/api/health`, then switches to **Remote Mode**.

### What Works in Remote Mode

| Feature | Status | Notes |
|---------|--------|-------|
| Chat | ✅ | Messages route through HTTP/SSE proxy |
| Tool execution | ✅ | Runs on the remote server's filesystem |
| Settings & Secrets | ✅ | Pushed securely to remote via proxy |
| Routines | ✅ | Execute on the remote server |
| Memory/Workspace | ✅ | File paths refer to the server's filesystem |
| Multiple profiles | ✅ | Switch between local and multiple remotes |

### Agent Profiles

Scrappy stores gateway profiles in `identity.json`:
```
~/Library/Application Support/OpenClaw/identity.json
```

Key fields:
- `gateway_mode`: `"local"` or `"remote"`
- `remote_url`: `"http://192.168.1.50:18789"`
- `remote_token`: `"<auth-token>"`
- `profiles`: Array of `{ id, name, mode, url, token }`

---

## Securing the Connection (Tailscale)

**⚠️ The gateway uses plain HTTP by default. Do NOT expose port 18789 to the public internet without encryption.**

The easiest and most secure approach is **Tailscale** (free mesh VPN):

### Step 1: Install Tailscale on Both Machines

```bash
# macOS (either machine)
brew install tailscale
# Or download from https://tailscale.com/download/mac

# Linux
curl -fsSL https://tailscale.com/install.sh | sh
```

### Step 2: Authenticate

```bash
# On both machines
sudo tailscale up
```

### Step 3: Find the Server's Tailscale IP

```bash
# On the ThinClaw server (Mac Mini / VPS)
tailscale ip -4
# Example output: 100.64.0.2
```

### Step 4: Connect Scrappy via Tailscale

In Scrappy's **Connect Existing** dialog:
- **Gateway URL:** `http://100.64.0.2:18789`

### Step 5: (Optional) Restrict to Tailscale Only

On the ThinClaw server, bind the gateway to the Tailscale interface only:

```env
# ~/.thinclaw/.env
GATEWAY_HOST=100.64.0.2   # Tailscale IP instead of 0.0.0.0
```

Or keep `0.0.0.0` but use a firewall:

```bash
# macOS: Allow only via Tailscale
sudo pfctl -e
echo "block in on en0 proto tcp from any to any port 18789" | sudo pfctl -f -

# Linux: UFW
sudo ufw delete allow 18789/tcp
sudo ufw allow in on tailscale0 to any port 18789 proto tcp
```

### What Tailscale Gives You

- ✅ End-to-end WireGuard encryption
- ✅ Zero public open ports
- ✅ Works from anywhere (coffee shop, travel, mobile hotspot)
- ✅ No port-forwarding or dynamic DNS needed
- ✅ MagicDNS: use `mac-mini` instead of IP addresses

### Alternative: Reverse Proxy with SSL

If you prefer not to use Tailscale, set up a reverse proxy:

```nginx
# /etc/nginx/sites-available/thinclaw
server {
    listen 443 ssl;
    server_name thinclaw.yourdomain.com;

    ssl_certificate     /etc/letsencrypt/live/thinclaw.yourdomain.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/thinclaw.yourdomain.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:18789;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_buffering off;           # Required for SSE
        proxy_cache off;
    }
}
```

Then connect Scrappy to `https://thinclaw.yourdomain.com`.

---

## Auto-Start on Boot

### macOS: launchd

ThinClaw has a built-in service manager that generates a macOS launchd plist and manages it for you. This is the **recommended** approach.

> **Important: Complete onboarding first.** The service runs headless with `--no-onboard` — the interactive setup wizard cannot run in a background process. You **must** run `thinclaw` (or `thinclaw onboard`) in a terminal first to complete the setup wizard before installing the service.

**Step 1: Complete onboarding (interactive terminal required)**

```bash
# First run — the setup wizard configures database, LLM, channels, etc.
thinclaw
# (or: thinclaw onboard)
```

**Step 2: Install and start the service**

```bash
# Install the launchd service (creates ~/Library/LaunchAgents/com.thinclaw.daemon.plist)
thinclaw service install

# Start the service (loads the plist and starts ThinClaw in the background)
thinclaw service start

# Check if it's running
thinclaw service status

# Stop the service
thinclaw service stop

# Remove the service entirely
thinclaw service uninstall
```

**What this does:**

- Creates `~/Library/LaunchAgents/com.thinclaw.daemon.plist` pointing to the current `thinclaw` binary
- Runs `thinclaw run --no-onboard` — starts the agent with your existing configuration, skipping the wizard
- Sets `RunAtLoad=true` — ThinClaw starts automatically when you log in (or on boot if auto-login is enabled)
- Sets `KeepAlive=true` — macOS automatically restarts ThinClaw if it crashes or exits
- Logs go to `~/.thinclaw/logs/daemon.stdout.log` and `~/.thinclaw/logs/daemon.stderr.log`
- Starts **all configured channels** (Telegram, iMessage, Discord, Gateway, etc.)

> **Note:** The service runs the exact same binary with the exact same channels as `thinclaw run` in a terminal. The only difference is that launchd manages the process lifecycle (auto-start on boot, auto-restart on crash) instead of your terminal session.

**View logs:**

```bash
tail -f ~/.thinclaw/logs/daemon.stdout.log
tail -f ~/.thinclaw/logs/daemon.stderr.log
```

<details>
<summary>Manual plist (advanced — only if you need custom configuration)</summary>

If you need to customize the plist beyond what `thinclaw service install` provides (e.g., environment variables, working directory), you can create it manually:

```bash
cat > ~/Library/LaunchAgents/com.thinclaw.daemon.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.thinclaw.daemon</string>

    <key>ProgramArguments</key>
    <array>
        <string>/Users/yourname/ThinClaw/target/release/thinclaw</string>
        <string>run</string>
    </array>

    <key>WorkingDirectory</key>
    <string>/Users/yourname/ThinClaw</string>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>/Users/yourname/.thinclaw/logs/daemon.stdout.log</string>

    <key>StandardErrorPath</key>
    <string>/Users/yourname/.thinclaw/logs/daemon.stderr.log</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>/Users/yourname</string>
    </dict>
</dict>
</plist>
EOF
```

**Replace `/Users/yourname` with your actual home directory.**

Manage manually:

```bash
launchctl load -w ~/Library/LaunchAgents/com.thinclaw.daemon.plist
launchctl start com.thinclaw.daemon
launchctl stop com.thinclaw.daemon
launchctl unload -w ~/Library/LaunchAgents/com.thinclaw.daemon.plist
```

> **Important:** Use the label `com.thinclaw.daemon` to match `thinclaw service` commands. Using a different label will result in two conflicting service entries.

</details>

### Linux: systemd

For Docker-based deployments, `setup.sh --systemd` creates the service automatically. For direct binary:

```ini
# /etc/systemd/system/thinclaw.service
[Unit]
Description=ThinClaw AI Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=thinclaw
WorkingDirectory=/opt/thinclaw
ExecStart=/opt/thinclaw/thinclaw run --no-onboard
Restart=always
RestartSec=10
Environment=HOME=/home/thinclaw

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable thinclaw
sudo systemctl start thinclaw

# View logs
journalctl -u thinclaw -f
```

---

## macOS-Specific Features

When ThinClaw runs on macOS (e.g., a Mac Mini), it gains access to capabilities unavailable on Linux/Docker:

| Feature | How to Enable | Notes |
|---------|--------------|-------|
| **macOS Keychain** | Choose "Keychain" in Step 2 of onboarding | Secrets encrypted by macOS at rest (AES-256). Never touch disk as plaintext |
| **iMessage Channel** | Set `IMESSAGE_ALLOW_FROM=+1234567890` | Read and reply to iMessages. Requires Full Disk Access permission |
| **Metal GPU** | Automatic on Apple Silicon | Unified memory for local model inference (MLX, llama.cpp). M1-M4 supported |
| **Camera Capture** | Built-in `camera_capture` tool | Agent can take photos if camera connected. Requires user permission grant |
| **Screen Capture** | Built-in `screen_capture` tool | Agent can screenshot the desktop. Requires Screen Recording permission |
| **Text-to-Speech** | Built-in `tts` tool | Uses macOS `say` command or OpenAI TTS |
| **Accessibility** | Grant in System Settings | Enables UI automation tools |

### macOS Permissions

On first use, macOS will prompt for the following permissions in **System Settings → Privacy & Security**:

- **Full Disk Access** — Required for iMessage channel (reads `chat.db`)
- **Screen Recording** — Required for `screen_capture` tool
- **Camera** — Required for `camera_capture` tool
- **Accessibility** — Required for UI automation
- **Keychain Access** — Prompted automatically when using Keychain-backed secrets

---

## Environment Reference

Comprehensive list of all environment variables. Set these in `~/.thinclaw/.env` or export them.

### Required

| Variable | Example | Description |
|----------|---------|-------------|
| `LLM_BACKEND` | `anthropic` | LLM provider: `anthropic`, `openai`, `ollama`, `openai_compatible`, `tinfoil` |
| API key for chosen provider | `sk-ant-...` | `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `LLM_API_KEY`, etc. |

### Gateway (Required for Scrappy)

| Variable | Default | Description |
|----------|---------|-------------|
| `GATEWAY_ENABLED` | `true` | Enable the HTTP/SSE/WS gateway |
| `GATEWAY_HOST` | `127.0.0.1` | Bind address. Use `0.0.0.0` for remote access |
| `GATEWAY_PORT` | `3000` | Port number. Docker deployments use `18789` |
| `GATEWAY_AUTH_TOKEN` | *(none)* | Bearer token for API authentication |

### Database

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_BACKEND` | `postgres` | `postgres` or `libsql` |
| `DATABASE_URL` | *(none)* | PostgreSQL connection string |
| `LIBSQL_PATH` | `~/.thinclaw/thinclaw.db` | LibSQL local file path |
| `LIBSQL_URL` | *(none)* | Turso cloud sync URL |
| `LIBSQL_AUTH_TOKEN` | *(none)* | Turso auth token |

### Agent

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENT_NAME` | `thinclaw` | Agent display name |
| `CLI_ENABLED` | `true` | Enable REPL mode. Set `false` for headless |
| `MAX_PARALLEL_JOBS` | `5` | Max concurrent jobs |
| `AGENT_AUTO_APPROVE_TOOLS` | `false` | Auto-approve tool calls (dangerous!) |

### Channels

| Variable | Description |
|----------|-------------|
| `TELEGRAM_BOT_TOKEN` | Telegram bot token |
| `TELEGRAM_OWNER_ID` | Telegram owner user ID |
| `DISCORD_BOT_TOKEN` | Discord bot token |
| `DISCORD_GUILD_ID` | Discord guild (server) ID |
| `SIGNAL_HTTP_URL` | Signal-CLI REST API URL |
| `SIGNAL_ALLOW_FROM` | Allowed Signal phone numbers |
| `IMESSAGE_ALLOW_FROM` | Allowed iMessage contacts (macOS only) |
| `NOSTR_PRIVATE_KEY` | Nostr private key |
| `NOSTR_RELAYS` | Comma-separated Nostr relay URLs |

### Embeddings

| Variable | Default | Description |
|----------|---------|-------------|
| `EMBEDDING_ENABLED` | `false` | Enable semantic memory search |
| `EMBEDDING_PROVIDER` | *(none)* | `openai` or `ollama` |
| `OPENAI_API_KEY` | *(none)* | Required for OpenAI embeddings |
| `EMBEDDING_MODEL` | `text-embedding-3-small` | Embedding model |

### Optional Features

| Variable | Default | Description |
|----------|---------|-------------|
| `HEARTBEAT_ENABLED` | `false` | Periodic background execution |
| `HEARTBEAT_INTERVAL_SECS` | `1800` | Heartbeat interval (30 min) |
| `SANDBOX_ENABLED` | `false` | Docker sandbox for tool execution |
| `ROUTINES_ENABLED` | `true` | Cron/event/webhook routines |

See `CLAUDE.md` for the complete configuration reference.

---

## Troubleshooting

### Connection Issues

| Issue | Solution |
|-------|---------|
| `curl: Connection refused` on port 18789 | Ensure `GATEWAY_HOST=0.0.0.0` (not `127.0.0.1`). Verify ThinClaw is running |
| Scrappy "Test & Save" fails | Check the URL includes the port. Verify auth token matches. Test with `curl` first |
| Can't reach server from another machine | Check firewall rules. Use `curl http://<ip>:18789/api/health` from the client |
| Works on LAN but not remotely | Use Tailscale or set up port forwarding / reverse proxy |

### macOS Issues

| Issue | Solution |
|-------|---------|
| Keychain dialog keeps appearing | Normal on first run — click "Allow Always". ThinClaw caches after first access |
| iMessage channel not working | Grant Full Disk Access in System Settings → Privacy → Full Disk Access |
| Metal inference slow | Check model is fully loaded to GPU. Adjust memory allocation in Settings |
| Agent stops when closing Terminal | Use launchd (see Auto-Start section) or run in `tmux`/`screen` |

### General Issues

| Issue | Solution |
|-------|---------|
| `ONBOARD_COMPLETED` not set | Run `thinclaw onboard` to re-run the wizard |
| Agent won't start after crash | Check `~/.thinclaw/thinclaw.log`. Run `thinclaw doctor` for diagnostics |
| Missing tools | Run `thinclaw tool list` to see available tools. Re-run onboarding Step 7 |
| Database errors | Run `thinclaw doctor` — it checks DB connectivity and migrations |

### CLI Quick Reference

```bash
# Start agent
thinclaw run

# Start with specific config
thinclaw run --config /path/to/config.toml

# Re-run onboarding wizard
thinclaw onboard

# Diagnostics
thinclaw doctor
thinclaw status

# Log management
thinclaw logs tail

# Service management
thinclaw service install
thinclaw service start
thinclaw service stop
thinclaw service status

# Tool management
thinclaw tool list
thinclaw tool install <name>

# Update binary
thinclaw update
```

---

## Gateway API Reference

The standalone gateway exposes a full REST API:

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/api/health` | GET | No | Health check |
| `/api/chat/send` | POST | Yes | Send a message |
| `/api/chat/events` | GET | Yes | SSE event stream |
| `/api/chat/ws` | GET | Yes | WebSocket upgrade |
| `/api/chat/threads` | GET | Yes | List sessions |
| `/api/chat/history` | GET | Yes | Get chat history |
| `/api/memory/read` | GET | Yes | Read workspace file |
| `/api/memory/write` | POST | Yes | Write workspace file |
| `/api/memory/search` | POST | Yes | Search workspace |
| `/api/gateway/status` | GET | Yes | Full status + diagnostics |
| `/api/settings/{key}` | GET/PUT | Yes | Read/write settings |
| `/api/routines` | GET | Yes | List routines |
| `/api/skills` | GET | Yes | List skills |
| `/api/extensions` | GET | Yes | List extensions |
| `/v1/chat/completions` | POST | Yes | OpenAI-compatible endpoint |

Authentication: `Authorization: Bearer <GATEWAY_AUTH_TOKEN>` header.

The gateway also serves a **built-in web UI** at the root URL (`http://<ip>:18789/`) — you can chat with the agent, manage memory, view logs, and configure settings directly in any browser.

---

## Comparison Matrix

| Feature | Scrappy Desktop | Direct Binary | Docker Compose |
|---------|:---:|:---:|:---:|
| **Network API** | ❌ | ✅ Gateway | ✅ Gateway |
| **Telegram / Discord** | ❌ | ✅ | ✅ |
| **iMessage** | ❌ | ✅ (macOS) | ❌ |
| **macOS Keychain** | ✅ | ✅ (macOS) | ❌ |
| **Settings UI** | ✅ Scrappy | ✅ Web UI | ✅ Web UI |
| **Metal GPU** | ✅ | ✅ (macOS) | ❌ |
| **Database** | LibSQL (auto) | LibSQL / PG | LibSQL / PG |
| **Auto-start** | App launch | launchd / systemd | docker restart |
| **OS requirement** | macOS | Any | Any (Docker) |
| **Difficulty** | Easiest | Easy-Medium | Easy |
