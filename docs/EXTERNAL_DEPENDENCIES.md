# ThinClaw External Dependencies Guide

This document lists every **optional external tool and service** that ThinClaw features may depend on, with direct links to download pages, install commands, and how ThinClaw uses each one.

> **Core ThinClaw has zero external runtime dependencies.** The binary is self-contained. Everything listed here is optional and unlocks specific features.

---

## Table of Contents

- [Quick Reference Matrix](#quick-reference-matrix)
- [VPN / Tunnel Providers](#vpn--tunnel-providers)
  - [Tailscale](#tailscale)
  - [Cloudflare Tunnel](#cloudflare-tunnel)
  - [ngrok](#ngrok)
  - [Custom Tunnel](#custom-tunnel)
- [Container Runtimes](#container-runtimes)
  - [Docker](#docker)
  - [Podman](#podman)
- [Browser Automation](#browser-automation)
  - [Chrome / Chromium (local)](#chrome--chromium-local)
  - [Docker Chromium (headless)](#docker-chromium-headless)
- [Messaging Channel Dependencies](#messaging-channel-dependencies)
  - [signal-cli (Signal channel)](#signal-cli-signal-channel)
  - [ffmpeg / ffprobe (media processing)](#ffmpeg--ffprobe-media-processing)
- [Code Delegation](#code-delegation)
  - [Claude Code CLI](#claude-code-cli)
- [Speech / Audio](#speech--audio)
  - [Whisper-compatible STT endpoint](#whisper-compatible-stt-endpoint)
- [Databases](#databases)
  - [PostgreSQL + pgvector](#postgresql--pgvector)
  - [libSQL / Turso](#libsql--turso)
- [Local Inference Engines](#local-inference-engines)
  - [Ollama](#ollama)
  - [llama.cpp](#llamacpp)
- [Reverse Proxies (for TLS)](#reverse-proxies-for-tls)
  - [nginx](#nginx)
  - [Caddy](#caddy)
- [Build-Time Dependencies](#build-time-dependencies)

---

## Quick Reference Matrix

| Dependency | ThinClaw Feature | Required? | Free Tier? | Install |
|-----------|-----------------|-----------|-----------|---------|
| [Tailscale](#tailscale) | VPN mesh, Tailscale tunnel, secure remote access | Optional | ✅ Free (100 devices) | `brew install tailscale` / [tailscale.com](https://tailscale.com/download) |
| [Cloudflare Tunnel](#cloudflare-tunnel) | Public webhook endpoints | Optional | ✅ Free | `brew install cloudflared` / [developers.cloudflare.com](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/) |
| [ngrok](#ngrok) | Public webhook endpoints | Optional | ✅ Free (limited) | `brew install ngrok` / [ngrok.com](https://ngrok.com/download) |
| [Docker](#docker) | Sandbox jobs, worker containers, Docker Compose deploy | Optional | ✅ Free | `brew install docker` / [docs.docker.com](https://docs.docker.com/get-docker/) |
| [Podman](#podman) | Rootless sandbox alternative to Docker | Optional | ✅ Free (OSS) | `brew install podman` / [podman.io](https://podman.io/docs/installation) |
| [Chrome/Chromium](#chrome--chromium-local) | `BrowserTool` web automation | Optional | ✅ Free | [google.com/chrome](https://www.google.com/chrome/) / `brew install --cask chromium` |
| [signal-cli](#signal-cli-signal-channel) | Signal messaging channel | Optional | ✅ Free (OSS) | [github.com/AsamK/signal-cli](https://github.com/AsamK/signal-cli) |
| [ffmpeg](#ffmpeg--ffprobe-media-processing) | Video/audio media processing | Optional | ✅ Free (OSS) | `brew install ffmpeg` / `apt install ffmpeg` |
| [PostgreSQL](#postgresql--pgvector) | Production database with vector search | Optional | ✅ Free (OSS) | `brew install postgresql@15` / `apt install postgresql` |
| [Ollama](#ollama) | Local LLM inference | Optional | ✅ Free (OSS) | [ollama.com](https://ollama.com/download) |
| [llama.cpp](#llamacpp) | Local LLM inference (server mode) | Optional | ✅ Free (OSS) | [github.com/ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) |
| [Claude Code](#claude-code-cli) | Delegated coding tasks in sandbox | Optional | API key required | `npm install -g @anthropic-ai/claude-code` / [docs.anthropic.com](https://docs.anthropic.com/en/docs/claude-code) |
| [Whisper endpoint](#whisper-compatible-stt-endpoint) | Audio transcription / talk mode | Optional | Varies | OpenAI API or local whisper.cpp server |
| [nginx](#nginx) / [Caddy](#caddy) | TLS termination (reverse proxy) | Optional | ✅ Free (OSS) | `brew install nginx` / `brew install caddy` |

Windows host defaults:
- Docker-backed sandbox and browser fallback expect Docker Desktop.
- Local browser automation auto-detects Chrome, Edge, and Brave.
- Camera/microphone capture expect `ffmpeg`, and Signal attachments can be forced with `SIGNAL_ATTACHMENTS_DIR`.
- Secrets use the Windows OS secure store; service management uses the Windows Service Control Manager.

---

## VPN / Tunnel Providers

ThinClaw supports managed tunnel integrations when the relevant feature/profile is enabled. Tunnel support should be treated as an optional deployment capability, not as something every default build automatically starts.

### Tailscale

**What it does:** Encrypted WireGuard mesh VPN. ThinClaw uses it for:
1. **Secure remote access** — Connect Scrappy to ThinClaw over an encrypted tunnel without exposing ports to the internet
2. **Tailscale Serve** — Expose the gateway to your tailnet (private)
3. **Tailscale Funnel** — Expose the gateway to the public internet (for webhooks)

**Where to get it:**

| Platform | Install Command | Download Page |
|----------|----------------|---------------|
| macOS | `brew install tailscale` | [tailscale.com/download/mac](https://tailscale.com/download/mac) |
| Linux | `curl -fsSL https://tailscale.com/install.sh \| sh` | [tailscale.com/download/linux](https://tailscale.com/download/linux) |
| Windows | — | [tailscale.com/download/windows](https://tailscale.com/download/windows) |

**ThinClaw configuration:**

```env
# Use Tailscale as managed tunnel provider
TUNNEL_PROVIDER=tailscale

# Use Tailscale Funnel (public internet) instead of Serve (tailnet-only)
TUNNEL_TS_FUNNEL=true

# Override auto-detected hostname
TUNNEL_TS_HOSTNAME=my-agent
```

Or simply use Tailscale as a VPN and set the gateway to bind to the Tailscale IP:

```env
GATEWAY_HOST=100.64.0.2   # Your Tailscale IP
```

**Verify:**

```bash
tailscale status            # Check connection
tailscale ip -4             # Get your Tailscale IP
```

> **Pricing:** Free for personal use (up to 100 devices). See [tailscale.com/pricing](https://tailscale.com/pricing).

---

### Cloudflare Tunnel

**What it does:** Creates a secure outbound tunnel from your machine to Cloudflare's edge network. No open inbound ports needed.

**Where to get it:**

| Platform | Install Command | Download Page |
|----------|----------------|---------------|
| macOS | `brew install cloudflared` | [developers.cloudflare.com](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/) |
| Linux (deb) | `sudo apt install cloudflared` | Same link above |
| Linux (rpm) | `sudo yum install cloudflared` | Same link above |

**ThinClaw configuration (managed):**

```env
# Managed tunnel — ThinClaw starts/stops cloudflared automatically
TUNNEL_PROVIDER=cloudflare
TUNNEL_CF_TOKEN=your-tunnel-token-from-zero-trust-dashboard
```

**ThinClaw configuration (manual):**

```env
# Manual tunnel — you run cloudflared yourself
TUNNEL_URL=https://your-tunnel.trycloudflare.com
```

**Quick start (manual, no account needed):**

```bash
# Zero-config tunnel (generates a random URL, no Cloudflare account required)
cloudflared tunnel --url http://localhost:8080
```

> **Pricing:** Cloudflare Tunnels are free. Zero Trust is free for up to 50 users. See [cloudflare.com/plans](https://www.cloudflare.com/plans/).

---

### ngrok

**What it does:** Instant public HTTPS URLs that forward to your local agent. Popular for webhook development.

**Where to get it:**

| Platform | Install Command | Download Page |
|----------|----------------|---------------|
| macOS | `brew install ngrok` | [ngrok.com/download](https://ngrok.com/download) |
| Linux | `snap install ngrok` | Same link above |
| Any | `curl -sSL https://ngrok-agent.s3.amazonaws.com/ngrok.asc \| ...` | Same link above |

**ThinClaw configuration:**

```env
TUNNEL_PROVIDER=ngrok
TUNNEL_NGROK_TOKEN=your-ngrok-authtoken

# Optional: use a custom domain (requires paid ngrok plan)
TUNNEL_NGROK_DOMAIN=agent.yourdomain.com
```

**Get your auth token:** Sign up at [dashboard.ngrok.com](https://dashboard.ngrok.com/signup), then copy your token from the [dashboard](https://dashboard.ngrok.com/get-started/your-authtoken).

> **Pricing:** Free tier includes 1 agent, 1 domain. See [ngrok.com/pricing](https://ngrok.com/pricing).

---

### Custom Tunnel

**What it does:** Wraps any tunnel binary behind ThinClaw's lifecycle management. Use this for tools like [bore](https://github.com/ekzhang/bore), [frp](https://github.com/fatedier/frp), or any custom solution.

**ThinClaw configuration:**

```env
TUNNEL_PROVIDER=custom

# Shell command — {port} and {host} are replaced at runtime
TUNNEL_CUSTOM_COMMAND=bore local {port} --to bore.pub

# Optional: poll this URL to check tunnel health
TUNNEL_CUSTOM_HEALTH_URL=https://your-tunnel-url.example.com/api/health

# Optional: look for this substring in stdout to extract the public URL
TUNNEL_CUSTOM_URL_PATTERN=https://
```

---

## Container Runtimes

### Docker

**What it does:** ThinClaw uses Docker for:
1. **Docker Sandbox** — Isolated container execution for untrusted code (orchestrator/worker pattern)
2. **Worker Image** — Full dev toolchain container (Rust, Node.js, Python, Git, Claude Code)
3. **Docker Compose Deployment** — Deploy ThinClaw as a containerized service
4. **Docker Chromium** — Headless browser in a container (fallback when no local Chrome)

**Where to get it:**

| Platform | Install Command | Download Page |
|----------|----------------|---------------|
| macOS | `brew install --cask docker` | [docs.docker.com/desktop/mac](https://docs.docker.com/desktop/install/mac-install/) |
| Ubuntu/Debian | See script below | [docs.docker.com/engine/install](https://docs.docker.com/engine/install/) |
| Other Linux | See link | Same link above |
| Windows | `winget install Docker.DockerDesktop` | [docs.docker.com/desktop/windows](https://docs.docker.com/desktop/install/windows-install/) |

**Linux quick install:**

```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER
# Log out and back in for group membership to take effect
```

**ThinClaw configuration:**

```env
# Enable Docker sandbox for isolated job execution
SANDBOX_ENABLED=true

# Resource limits for worker containers
SANDBOX_MEMORY_LIMIT_MB=2048
SANDBOX_CPU_SHARES=1024

# Extra domains the sandbox proxy should allow
SANDBOX_EXTRA_DOMAINS=api.openai.com,api.anthropic.com
```

**Build the worker image (required before first sandbox use):**

```bash
docker build -f Dockerfile.worker -t thinclaw-worker .
```

The onboarding wizard offers to build this automatically during worker sandbox setup.

**Verify:**

```bash
docker --version            # Should show Docker version
docker info                 # Verify daemon is running
docker run --rm hello-world # Quick sanity check
```

---

### Podman

**What it does:** Rootless, daemonless alternative to Docker. ThinClaw supports Podman as a sandbox runtime.

**Where to get it:**

| Platform | Install Command | Download Page |
|----------|----------------|---------------|
| macOS | `brew install podman` | [podman.io/docs/installation](https://podman.io/docs/installation) |
| Ubuntu/Debian | `sudo apt install podman` | Same link above |
| Fedora/RHEL | `sudo dnf install podman` | Same link above |

**ThinClaw configuration:**

```env
# Use Podman instead of Docker for sandbox
CONTAINER_RUNTIME=podman
```

> Podman runs containers as your user (rootless by default) — no daemon required.

---

## Browser Automation

### Chrome / Chromium (local)

**What it does:** The `BrowserTool` uses Chrome-family browsers for web automation (navigating, screenshots, link extraction, text extraction).

**Where to get it:**

| Option | Install | Notes |
|--------|---------|-------|
| Google Chrome | [google.com/chrome](https://www.google.com/chrome/) | Most common; auto-detected |
| Microsoft Edge | Built into most Windows installs | Auto-detected on Windows |
| Brave | [brave.com/download](https://brave.com/download/) | Auto-detected on macOS, Linux, and Windows |
| Chromium (OSS) | `brew install --cask chromium` (macOS) | Lighter, open-source |
| Chromium (Linux) | `apt install chromium-browser` | Package name varies by distro |

ThinClaw auto-detects the local browser binary in common locations. No configuration is needed if Chrome, Edge, or Brave is installed normally.

**Override the binary path:**

```env
BROWSER_EXECUTABLE=/usr/bin/google-chrome-stable
```

`CHROME_PATH` is still accepted as a legacy alias, but new Linux deployments
should use `BROWSER_EXECUTABLE`.

---

### Docker Chromium (headless)

**What it does:** When no local browser is found (or on headless servers), ThinClaw automatically starts a Docker container with headless Chromium for browser automation.

**Prerequisites:** Docker Desktop on Windows, or Docker on macOS/Linux (see [Docker section](#docker))

**ThinClaw configuration:**

```env
# auto: use local browser first, then Docker Chromium fallback
# always: force Docker Chromium even if a local binary exists
# never: disable Docker Chromium fallback
BROWSER_DOCKER=auto
CHROMIUM_IMAGE=chromedp/headless-shell:latest
```

The default image is public and multi-arch (`linux/amd64` and `linux/arm64`).
Set `CHROMIUM_IMAGE` only if you operate an internal CDP-capable Chromium image.
The `BrowserTool` handles the container lifecycle automatically (pull, start,
health-check, stop), and `thinclaw doctor` verifies that the image is local or
pullable before reporting the fallback as ready.

---

## Linux Runtime Readiness

Use `thinclaw doctor --profile server` for Ubuntu/Debian server or Docker hosts,
`thinclaw doctor --profile pi-os-lite-64` for Raspberry Pi OS Lite 64-bit,
`thinclaw doctor --profile desktop-linux` for the supported interactive Linux
desktop path, and `thinclaw doctor --profile all-features` before building or
running with every optional feature enabled. `desktop-gnome` remains accepted as
a compatibility alias.

Raspberry Pi OS Lite 64-bit supports the full headless runtime. Docker, Tailscale,
camera, microphone, location, and browser automation are optional. Reckless
desktop autonomy is not supported on Pi OS Lite; keep `DESKTOP_AUTONOMY_ENABLED=false`.

Pi OS Lite quick checks:

```bash
uname -m                    # expected: aarch64
cat /etc/os-release         # expected: Raspberry Pi OS / Debian bookworm
systemctl --version
thinclaw doctor --profile pi-os-lite-64
```

Pi OS Lite native service defaults:

```env
THINCLAW_HOME=/var/lib/thinclaw/.thinclaw
THINCLAW_RUNTIME_PROFILE=pi-os-lite-64
THINCLAW_HEADLESS=true
DATABASE_BACKEND=libsql
LIBSQL_PATH=/var/lib/thinclaw/.thinclaw/thinclaw.db
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=3000
THINCLAW_ALLOW_ENV_MASTER_KEY=1
DESKTOP_AUTONOMY_ENABLED=false
```

The `THINCLAW_RUNTIME_PROFILE=pi-os-lite-64` and `THINCLAW_HEADLESS=true`
markers are runtime guards: desktop autonomy tools are not registered under
that profile, so Pi OS Lite remains a remote/headless setup rather than a
desktop automation host.

Optional Pi packages:

```bash
# Useful baseline for source builds and diagnostics
sudo apt-get install -y build-essential curl git pkg-config ca-certificates

# Optional browser/sandbox fallback
sudo apt-get install -y docker.io docker-compose-plugin
sudo systemctl enable --now docker

# Optional private network access
curl -fsSL https://tailscale.com/install.sh | sh
```

Important Linux env vars:

```env
BROWSER_EXECUTABLE=/usr/bin/google-chrome-stable
BROWSER_DOCKER=auto
CHROMIUM_IMAGE=chromedp/headless-shell:latest
SCREEN_CAPTURE_ENABLED=false
CAMERA_CAPTURE_ENABLED=false
TALK_MODE_ENABLED=false
LOCATION_ENABLED=false
LOCATION_ALLOW_IP_FALLBACK=false
THINCLAW_CAMERA_DEVICE=/dev/video0
THINCLAW_MICROPHONE_DEVICE=default
THINCLAW_MICROPHONE_BACKEND=auto
THINCLAW_ALLOW_ENV_MASTER_KEY=1
SECRETS_MASTER_KEY=hex-encoded-32-byte-key-for-headless-hosts
DESKTOP_AUTONOMY_ENABLED=false
```

`SECRETS_MASTER_KEY` is ignored by default. Use the environment fallback only for headless hosts or containers where Linux Secret Service is not available, and prefer a service-manager secret mechanism over plain shell exports.

Ubuntu/Debian packages for the supported interactive Linux desktop-autonomy path:

```bash
sudo apt install python3 python3-gi python3-pyatspi libreoffice \
  libreoffice-script-provider-python evolution evolution-data-server-bin \
  xdotool ydotool wmctrl tesseract-ocr gnome-screenshot scrot grim spectacle imagemagick \
  at-spi2-core libglib2.0-bin geoclue-2.0 ffmpeg fswebcam \
  kwin-wayland plasma-workspace plasma-workspace-wayland xwayland
```

Use `xdotool` for X11 sessions. Use `ydotool` or `dotool` when the host runs a
Wayland compositor that does not permit X11 input injection. `grim` and
`spectacle` provide Wayland-friendly screen capture paths; KDE/Plasma sessions
prefer `spectacle` because `grim` is compositor-protocol dependent.

Linux native Apple Mail and native iMessage channels are not available. Use
Gmail for mail and BlueBubbles for iMessage-compatible messaging from a Mac-hosted
BlueBubbles server.

---

## Messaging Channel Dependencies

### signal-cli (Signal channel)

**What it does:** The Signal channel uses [signal-cli](https://github.com/AsamK/signal-cli) in daemon/HTTP mode to send and receive Signal messages.

**Where to get it:**

| Method | Command / Link |
|--------|---------------|
| GitHub Releases | [github.com/AsamK/signal-cli/releases](https://github.com/AsamK/signal-cli/releases) |
| AUR (Arch) | `yay -S signal-cli` |
| Manual | Requires Java 21+ runtime |

**Quick setup:**

```bash
# 1. Download and extract signal-cli
# 2. Register or link your phone number:
signal-cli -a +1234567890 register
signal-cli -a +1234567890 verify CODE

# 3. Start in daemon mode (ThinClaw connects to this):
signal-cli -a +1234567890 daemon --http
```

**ThinClaw configuration:**

```env
SIGNAL_HTTP_URL=http://localhost:8080
SIGNAL_ALLOW_FROM=+1234567890,+0987654321
SIGNAL_ATTACHMENTS_DIR=/path/to/signal-cli/attachments   # Optional override, especially useful on Windows
```

See also: [signal-cli documentation](https://github.com/AsamK/signal-cli/wiki)

---

### ffmpeg / ffprobe (media processing)

**What it does:** ThinClaw uses ffmpeg/ffprobe for:
- Video keyframe extraction and metadata
- Audio extraction from video files
- Sticker conversion (WebP/TGS/WebM → image)

**Where to get it:**

| Platform | Install Command |
|----------|----------------|
| macOS | `brew install ffmpeg` |
| Ubuntu/Debian | `sudo apt install ffmpeg` |
| Fedora/RHEL | `sudo dnf install ffmpeg` |
| Windows | `winget install Gyan.FFmpeg` |

**Verify:**

```bash
ffmpeg -version
ffprobe -version
```

ThinClaw gracefully falls back when ffmpeg is not available — video processing features are simply disabled.

---

## Code Delegation

### Claude Code CLI

**What it does:** ThinClaw can delegate complex coding tasks to Anthropic's [Claude Code](https://docs.anthropic.com/en/docs/claude-code) CLI, running inside a sandboxed Docker container. The worker container includes Claude Code pre-installed.

**Prerequisites:** Docker (see [Docker section](#docker)) — Claude Code runs inside the worker container, not on the host.

**Where to get it (if running on host for testing):**

| Method | Command |
|--------|--------|
| npm | `npm install -g @anthropic-ai/claude-code` |
| Docs | [docs.anthropic.com/en/docs/claude-code](https://docs.anthropic.com/en/docs/claude-code) |

**Authentication** (checked in priority order):

| Priority | Source | How to Set |
|----------|--------|------------|
| 1 | `ANTHROPIC_API_KEY` env var | `export ANTHROPIC_API_KEY=sk-ant-api03-...` |
| 2 | OS secure store | Set during `thinclaw onboard` when Claude Code credentials are configured |
| 3 | Claude Code OAuth | Run `claude login` on the host machine |

**ThinClaw configuration:**

```env
CLAUDE_CODE_ENABLED=true
CLAUDE_CODE_MODEL=sonnet                # Claude model for code tasks
CLAUDE_CODE_MAX_TURNS=50                # Maximum agentic turns per job
CLAUDE_CODE_MEMORY_LIMIT_MB=4096        # Container memory limit
```

These can also be configured in the WebUI under **Settings → Features**.

---

## Speech / Audio

### Whisper-compatible STT endpoint

**What it does:** ThinClaw uses a Whisper-compatible HTTP endpoint for:
1. **Audio transcription** — Transcribe voice messages and audio attachments
2. **Talk mode** — Continuous voice conversation (STT → LLM → TTS pipeline)
3. **Push-to-talk** — Voice input from the Scrappy desktop app

**Options:**

| Option | Type | Free? | Link |
|--------|------|-------|------|
| OpenAI Whisper API | Cloud | Paid ($0.006/min) | [platform.openai.com](https://platform.openai.com/docs/guides/speech-to-text) |
| whisper.cpp server | Local | ✅ Free (OSS) | [github.com/ggml-org/whisper.cpp](https://github.com/ggml-org/whisper.cpp) |
| Scrappy sidecar | Local (desktop) | ✅ Free | Built into Scrappy desktop app |

**Quick start with whisper.cpp (local):**

```bash
git clone https://github.com/ggml-org/whisper.cpp.git
cd whisper.cpp && make
./models/download-ggml-model.sh base.en
./server -m models/ggml-base.en.bin --host 127.0.0.1 --port 8178
```

**ThinClaw configuration:**

```env
# Point to any Whisper-compatible HTTP endpoint
WHISPER_HTTP_ENDPOINT=http://127.0.0.1:8178/inference

# Or use OpenAI's cloud API (requires OPENAI_API_KEY):
WHISPER_HTTP_ENDPOINT=https://api.openai.com/v1/audio/transcriptions
```

If no Whisper endpoint is configured, audio transcription features are simply unavailable — the agent reports that transcription requires an endpoint.

---

## Databases

### PostgreSQL + pgvector

**What it does:** Production-grade database with native vector search for semantic memory.

**Where to get it:**

| Platform | Install Command | Download Page |
|----------|----------------|---------------|
| macOS | `brew install postgresql@15` | [postgresql.org/download](https://www.postgresql.org/download/) |
| Ubuntu/Debian | `sudo apt install postgresql postgresql-contrib` | Same link above |

**Install pgvector extension:**

```bash
# macOS (Homebrew)
brew install pgvector

# Ubuntu/Debian (from PGDG repo)
sudo apt install postgresql-15-pgvector

# Or build from source:
git clone https://github.com/pgvector/pgvector.git
cd pgvector && make && sudo make install
```

**Setup:**

```bash
createdb thinclaw
psql thinclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

**ThinClaw configuration:**

```env
DATABASE_BACKEND=postgres
DATABASE_URL=postgres://user:pass@localhost/thinclaw
```

> **Alternative:** Use **libSQL** (zero setup) — see below.

---

### libSQL / Turso

**What it does:** Embedded SQLite-compatible database. Zero dependencies, zero setup. Recommended for single-server / Mac Mini deployments.

**Where to get it:** Built into ThinClaw — no installation needed!

**ThinClaw configuration:**

```env
DATABASE_BACKEND=libsql
LIBSQL_PATH=~/.thinclaw/thinclaw.db

# Optional: sync to Turso cloud
LIBSQL_URL=libsql://your-db.turso.io
LIBSQL_AUTH_TOKEN=your-turso-token
```

For Turso cloud (optional): [turso.tech](https://turso.tech/)

---

## Local Inference Engines

### Ollama

**What it does:** Run LLMs locally on your machine. ThinClaw supports Ollama as both an LLM provider and an embedding provider.

**Where to get it:**

| Platform | Install Command | Download Page |
|----------|----------------|---------------|
| macOS | `brew install ollama` | [ollama.com/download](https://ollama.com/download) |
| Linux | `curl -fsSL https://ollama.com/install.sh \| sh` | Same link above |
| Windows | `winget install Ollama.Ollama` | Same link above |

**Quick start:**

```bash
ollama serve                    # Start the server
ollama pull llama3.1:8b         # Download a model
ollama pull nomic-embed-text    # Download embedding model
```

**ThinClaw configuration:**

```env
# LLM provider
LLM_BACKEND=ollama
OLLAMA_BASE_URL=http://localhost:11434   # default, optional
LLM_MODEL=llama3.1:8b

# Embedding provider
EMBEDDING_ENABLED=true
EMBEDDING_PROVIDER=ollama
EMBEDDING_MODEL=nomic-embed-text
```

---

### llama.cpp

**What it does:** High-performance local LLM inference with Metal GPU acceleration on Apple Silicon.

**Where to get it:**

| Method | Link |
|--------|------|
| GitHub | [github.com/ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) |
| Homebrew | `brew install llama.cpp` |
| Build | `git clone ... && make LLAMA_METAL=1` |

**Run the server:**

```bash
llama-server -m /path/to/model.gguf --host 0.0.0.0 --port 8080
```

**ThinClaw configuration:**

```env
LLM_BACKEND=llama_cpp
LLAMA_CPP_SERVER_URL=http://localhost:8080
```

---

## Reverse Proxies (for TLS)

ThinClaw's gateway uses **plain HTTP** by default. For production / internet-facing deployments, terminate TLS with a reverse proxy.

### nginx

**Where to get it:**

| Platform | Install Command |
|----------|----------------|
| macOS | `brew install nginx` |
| Ubuntu/Debian | `sudo apt install nginx` |

**Example config:**

```nginx
server {
    listen 443 ssl;
    server_name thinclaw.yourdomain.com;

    ssl_certificate     /etc/letsencrypt/live/thinclaw.yourdomain.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/thinclaw.yourdomain.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_buffering off;           # Required for SSE
        proxy_cache off;
    }
}
```

Use [Let's Encrypt](https://letsencrypt.org/) + [certbot](https://certbot.eff.org/) for free TLS certificates.

---

### Caddy

**Where to get it:**

| Platform | Install Command | Download Page |
|----------|----------------|---------------|
| macOS | `brew install caddy` | [caddyserver.com/docs/install](https://caddyserver.com/docs/install) |
| Ubuntu/Debian | See link | Same link above |

**Example Caddyfile:**

```
thinclaw.yourdomain.com {
    reverse_proxy localhost:3000
}
```

Caddy **automatically** provisions and renews TLS certificates from Let's Encrypt.

---

## Build-Time Dependencies

These are only needed if you **compile ThinClaw from source**:

| Dependency | Purpose | macOS | Linux | Windows / WSL |
|-----------|---------|-------|-------|---------------|
| Xcode CLI Tools | C compiler, linker | `xcode-select --install` | N/A | N/A |
| C/C++ build tools | Native dependency compilation | Xcode CLI Tools | `sudo apt install build-essential pkg-config` or distro equivalent | Use WSL and Linux packages for the supported Linux-style source path |
| Rust 1.92+ | Rust compiler | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` | Same | Install inside WSL for Linux-style builds |
| wasm32-wasip2 target | WASM compilation | `rustup target add wasm32-wasip2` | Same | Same inside WSL |
| wasm-tools | WASM component model | `cargo install wasm-tools --locked` | Same | Same inside WSL |
| cargo-component | Build WASM extensions | `cargo install cargo-component --locked` | Same | Same inside WSL |
| Git | Clone the repo | Pre-installed or Xcode CLI Tools | `sudo apt install git` or distro equivalent | Git for Windows or Git inside WSL |
| CA certificates and curl | Fetch installers/dependencies | Built in on most hosts | `sudo apt install ca-certificates curl` or distro equivalent | Built in PowerShell for release installs; install inside WSL for source builds |

**One-click setup scripts** (install all build dependencies automatically):

```bash
# macOS — release install:
./scripts/mac-deploy.sh

# macOS — source-build prerequisites only:
./scripts/mac-deploy.sh --from-source --install-only

# Developer setup (any OS):
./scripts/dev-setup.sh
```

Linux note: the core ThinClaw build now uses Rustls and does not require OpenSSL
development headers. If you enable the optional `voice` feature, also install
`libasound2-dev` so `cpal` can link against ALSA.

Windows native source builds are not the primary documented path. Prefer the MSI
or portable ZIP for native Windows, or use WSL 2 and follow the Linux source
build instructions when you need a development build on a Windows machine.

---

## Related Documentation

| Document | What It Covers |
|----------|---------------|
| [DEPLOYMENT.md](DEPLOYMENT.md) | Full deployment paths (binary, Docker Compose, automated) |
| [LLM_PROVIDERS.md](LLM_PROVIDERS.md) | LLM provider configuration details |
| [TELEGRAM_SETUP.md](TELEGRAM_SETUP.md) | Telegram bot setup and webhook/polling modes |
| [../src/NETWORK_SECURITY.md](../src/NETWORK_SECURITY.md) | Network security surface audit |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Development setup and contributing guidelines |
