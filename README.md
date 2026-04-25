<p align="center">
  <img src="Thinclaw_IC_01_nobg.png" alt="ThinClaw" width="180"/>
</p>

<h1 align="center">ThinClaw</h1>

<p align="center">
  <em>A self-hosted personal agent with a Rust runtime underneath</em>
</p>

<p align="center">
  <a href="https://github.com/RNT56/ThinClaw/releases"><img src="https://img.shields.io/github/v/release/RNT56/ThinClaw?style=flat-square&color=2ea44f&label=release" alt="Latest Release" /></a>
  &nbsp;
  <a href="https://github.com/RNT56/ThinClaw/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/RNT56/ThinClaw/ci.yml?branch=main&style=flat-square&label=CI" alt="CI" /></a>
  &nbsp;
  <a href="https://github.com/RNT56/ThinClaw/blob/main/LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue?style=flat-square" alt="License" /></a>
</p>

<p align="center">
  <a href="#quick-start"><img src="https://img.shields.io/badge/Quick_Start-grey?style=flat-square" alt="Quick Start" /></a>&nbsp;
  <a href="#why-thinclaw"><img src="https://img.shields.io/badge/Why_ThinClaw-grey?style=flat-square" alt="Why ThinClaw" /></a>&nbsp;
  <a href="#core-capabilities"><img src="https://img.shields.io/badge/Capabilities-grey?style=flat-square" alt="Capabilities" /></a>&nbsp;
  <a href="#deployment-modes"><img src="https://img.shields.io/badge/Deployment-grey?style=flat-square" alt="Deployment" /></a>&nbsp;
  <a href="#security-and-trust"><img src="https://img.shields.io/badge/Security-grey?style=flat-square" alt="Security" /></a>&nbsp;
  <a href="#documentation-map"><img src="https://img.shields.io/badge/Docs-grey?style=flat-square" alt="Docs" /></a>&nbsp;
  <a href="#development"><img src="https://img.shields.io/badge/Development-grey?style=flat-square" alt="Development" /></a>
</p>

---

## What Is ThinClaw?

ThinClaw is a Rust-based self-hosted agent you run yourself. It can operate as a standalone binary, a long-running service with a web gateway, or as the backend engine embedded inside Scrappy.

It is built around a few core ideas:

- a named agent with durable identity across CLI, WebUI, channels, and background work
- operator-controlled deployment, models, and integrations
- layered safety around secrets, tools, network access, and external content
- hybrid extensibility through native Rust, WASM, and MCP
- a proactive runtime built around channels, routines, memory, and background work
- an optional privileged desktop-autonomy profile for host-level app control and managed self-rollout

ThinClaw is not just a chat wrapper. It is an agent product built on a runtime that handles identity, sessions, tools, channels, persistence, and policy.

## Quick Start

macOS / Linux:

```bash
# 1. Install the latest release
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh

# 2. Run onboarding
thinclaw onboard
# or force the full-screen Humanist Cockpit shell:
# thinclaw onboard --ui tui
# or fully reset ThinClaw state and start over:
# thinclaw reset --yes

# 3. Start ThinClaw locally later
thinclaw
# or launch the full-screen runtime directly:
thinclaw tui

# 4. Open the gateway
# http://127.0.0.1:3000
```

Windows (PowerShell):

```powershell
# 1. Install the latest MSI or portable ZIP from GitHub Releases
# https://github.com/RNT56/ThinClaw/releases

# 2. Run onboarding
thinclaw onboard
# or fully reset ThinClaw state and start over:
# thinclaw reset --yes

# 3. Start ThinClaw locally later
thinclaw
# or launch the full-screen runtime directly:
thinclaw tui

# 4. Open the gateway
# http://127.0.0.1:3000
```

By default, `thinclaw` and `thinclaw run` use the same startup path and keep terminal output quiet, only surfacing warnings and errors during startup. If you want the full initialization log stream for troubleshooting, start it with either:

```bash
thinclaw --debug --no-onboard
thinclaw --debug run --no-onboard
```

If you need more targeted filtering, `RUST_LOG=...` still works and takes precedence.

For a deeper setup path, including service mode, remote access, provider guidance, Windows service management, and external dependencies, use the docs hub at [docs/README.md](docs/README.md).

The onboarding flow now uses a calmer "Humanist Cockpit" framing in both CLI and TUI modes, with shared readiness summaries, skin-aware presentation, saved follow-up notes, an explicit Quick Setup vs Advanced Setup split, and automatic handoff from onboarding into the matching local runtime.

Remote / SSH host setup:

```bash
thinclaw onboard --profile remote
thinclaw gateway access
thinclaw service install
thinclaw service start
```

This is the path for Raspberry Pi, Mac Mini, VPS, and other SSH-managed hosts.
It ships in the normal `thinclaw` release binary; there is no separate remote
artifact.

For Raspberry Pi OS Lite native service installs from the release artifact, use
the dedicated setup helper:

```bash
sudo bash deploy-setup.sh --mode native --binary ./thinclaw
```

## Why ThinClaw

### 1. Security Is Part of the Architecture

ThinClaw’s safety story is not one toggle. It is split across host-boundary secret injection, sandboxing, tool policy, network controls, and explicit trust boundaries.

- WASM tools and WASM channels are sandboxed and capability-scoped.
- Native channels and built-in tools run in the trusted host runtime.
- MCP servers are operator-trusted external processes or services, not sandboxed plugins.
- In restricted workspace modes, ThinClaw now avoids overstating execution isolation: background `process` is disabled outside unrestricted mode, `execute_code` only runs in `sandboxed` mode when the Docker sandbox is available, and research `local_docker` trials execute through the same Docker-backed runtime.
- On macOS, host-local no-network execution uses `sandbox-exec`; on Linux it now uses `bwrap` when available. Docker remains the portable hard-isolation path, and unsupported host-local platforms are reported honestly through runtime metadata instead of being implied as equivalent.
- Tool outputs and job surfaces now expose runtime backend, runtime family, runtime mode, capabilities, and network-isolation metadata end to end so operators do not have to infer trust and execution behavior from implementation details.
- `create_job`, including `worker`, `claude_code`, and `codex_code` modes, now executes through the same shared local-host / Docker execution backend family used by the rest of the runtime instead of a separate opaque path.
- Local research trials persist benchmark summaries into the experiment artifact store and restore the dedicated campaign worktree after each run, so benchmark byproducts do not contaminate later candidate diffs.

That distinction matters, and ThinClaw documents it explicitly instead of pretending every integration has the same trust model.

### 2. Hybrid Extensibility

ThinClaw uses different extension paths for different jobs:

- native Rust for persistent connections and local-system access
- WASM for hot-reloadable tools and channels with credential isolation
- MCP for external tool ecosystems where operator-managed trust is acceptable

This is a deliberate design choice, not a migration artifact.

### 3. Proactive Runtime

ThinClaw is designed for more than interactive chat:

- channels and web gateway delivery
- routines and background execution
- heartbeat and notifications
- workspace-backed memory and search
- subagents and multi-session orchestration

### 4. Flexible Deployment

You can run ThinClaw:

- locally on your own machine
- as a long-running service on macOS, Linux, or Windows
- behind the built-in gateway
- embedded inside Scrappy

## Host Support Matrix

| Host Surface | macOS | Linux | Windows |
|---|---|---|---|
| Local CLI / gateway host | Supported | Supported | Supported |
| Native OS secure store | Supported | Supported | Supported |
| `thinclaw service` lifecycle | Supported | Supported | Supported |
| Local browser automation | Chrome / Brave / Edge | Chrome / Chromium / Brave / Edge | Chrome / Edge / Brave |
| Docker browser fallback | Supported | Supported | Docker Desktop |
| Camera / microphone capture | Supported | Supported | Supported with `ffmpeg` |
| Signal attachments | Supported | Supported | Supported, override with `SIGNAL_ATTACHMENTS_DIR` when needed |
| Apple Mail / iMessage (native) | Supported | Unsupported | Unsupported |
| iMessage via BlueBubbles | Supported | Supported | Supported |

## Core Capabilities

- Multi-surface operation through the CLI, gateway, channels, and background jobs
- A shared identity model with `personality_pack` defaults for new workspaces and `/personality` session overlays (`/vibe` remains a compatibility alias)
- A shared command vocabulary centered on `/compress`, `/personality`, `/skills`, `/heartbeat`, `/summarize`, and `/rollback`
- Humanist Cockpit onboarding with shared CLI/TUI readiness framing, shared skin palettes, and saved follow-up notes
- Shared terminal skin system across boot, REPL, full-screen TUI, onboarding TUI, setup prompts, and human-readable CLI subcommands
- Built-in ASCII-art skins plus user-defined TOML skins from `~/.thinclaw/skins/`
- Hybrid delivery across native channels (Telegram, Signal, Discord, Slack, Nostr, Gmail, iMessage, BlueBubbles, Apple Mail) and packaged WASM channels, with platform formatting/rendering guidance owned by the channel layer instead of hard-coded in prompt assembly
- Workspace-backed memory with search, citations, and identity files
- Extension support through built-in tools, WASM tools, and MCP servers
- Multi-provider LLM routing, failover, and cost controls
- Operator-facing gateway UI for chat, memory, routines, logs, extensions, providers, and settings
- Operator-facing transparency controls for subagent detail levels and Telegram subagent session routing
- Optional reckless desktop autonomy with native app adapters, UI automation, evidence capture, and managed code autorollout/rollback

## Deployment Modes

| Situation | Best For | Start Here |
|---|---|---|
| macOS | local, Mac Mini, launchd service | [docs/deploy/macos.md](docs/deploy/macos.md) |
| Windows | native install, Windows service, WSL guidance | [docs/deploy/windows.md](docs/deploy/windows.md) |
| Linux | laptop, workstation, server, VPS | [docs/deploy/linux.md](docs/deploy/linux.md) |
| Raspberry Pi OS Lite 64-bit | Pi 4/5 or ARM64 Pi server | [docs/deploy/raspberry-pi-os-lite.md](docs/deploy/raspberry-pi-os-lite.md) |
| Docker | Compose or container deployment | [docs/deploy/docker.md](docs/deploy/docker.md) |
| Remote access | gateway, Tailscale, webhook tunnels | [docs/deploy/remote-access.md](docs/deploy/remote-access.md) |
| Reckless desktop autonomy | operator-approved host-level desktop automation | [docs/DESKTOP_AUTONOMY.md](docs/DESKTOP_AUTONOMY.md) |
| Scrappy embedding | local or remote ThinClaw runtime | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |

Code-backed local default: the gateway listens on port `3000` unless you configure otherwise.

For the full decision tree, use [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Build Profiles

ThinClaw uses Cargo feature flags to control binary size and capabilities.
The default build (`light`) is lean; opt into more with `--features`:

| Profile | Command | What It Adds |
|---|---|---|
| **light** (default) | `cargo build` | PostgreSQL, libSQL, local gateway, HTML-to-Markdown, doc extraction, timezones |
| **full** | `cargo build --release --features full` | + ACP, REPL/TUI, tunnel, Docker sandbox, browser, Nostr |
| **desktop** | `cargo build --features desktop` | libSQL, HTML-to-Markdown, doc extraction, REPL, timezones |
| **minimal** | `cargo build --no-default-features --features libsql` | Single DB backend, nothing else |

Additional opt-in flags not included in `full`: `voice`, `bedrock`, `bundled-wasm`.

GitHub Releases are the normal user path. The repo's release workflow publishes
the regular `thinclaw` binary with the `full` feature set for supported Linux,
macOS, and Windows targets.

Full details, custom combinations, and CI matrix: [docs/BUILD_PROFILES.md](docs/BUILD_PROFILES.md)

## Terminal Skins

Local terminal clients use the active CLI skin for palette, prompt symbol, tool labels, boot art, and human-readable command presentation. The WebUI now follows the active CLI skin by default and can optionally override it with a dedicated WebUI skin.

- Built-in skins: `cockpit`, `midnight`, `solar`, `athena`, `delphi`, `olympus`
- Runtime switching in local clients: `/skin`, `/skin list`, `/skin reset`
- Persistent default: `AGENT_CLI_SKIN=<name>`
- WebUI follow/override: `WEBCHAT_SKIN=<name>` or leave unset to follow `AGENT_CLI_SKIN`
- Custom skins: drop `name.toml` files into `~/.thinclaw/skins/`

Skin TOML files now support:

- core palette tokens: `accent`, `border`, `body`, `muted`, `good`, `warn`, `bad`, `header`
- prompt symbol: `prompt_symbol`
- skin-specific TUI hero art: `hero_art`
- optional skin subtitle: `tagline`
- tool label embellishments: `tool_emojis`
- optional WebUI aura colors: `[web].aura_primary`, `[web].aura_secondary`

Legacy compatibility note:

- `WEBCHAT_ACCENT_COLOR` still works, but it only retints accent surfaces in the WebUI and does not replace the shared skin identity, tagline, prompt symbol, or tool iconography

## Security And Trust

ThinClaw aims for operator control, but it does not claim that every configured integration is equally isolated.

- local data paths, secrets, and policy enforcement are handled in the trusted host runtime
- WASM components are sandboxed
- MCP servers, tunnels, LLM providers, and external services are real trust boundaries
- `desktop_autonomy.profile = "reckless_desktop"` is a privileged mode that adds host-level app/UI/screen control plus managed code promotion and rollback; treat it as a stronger trust grant than a normal local run

Use the deep docs before relying on a surface for sensitive workflows:

- [docs/SECURITY.md](docs/SECURITY.md)
- [docs/DESKTOP_AUTONOMY.md](docs/DESKTOP_AUTONOMY.md)
- [src/NETWORK_SECURITY.md](src/NETWORK_SECURITY.md)
- [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md)
- [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md)

## Documentation Map

Start here, then go deeper by topic:

- [docs/README.md](docs/README.md): audience-first docs index
- [docs/BUILD_PROFILES.md](docs/BUILD_PROFILES.md): build profiles, feature flags, and `full` vs `--all-features`
- [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md): deployment decision tree and platform runbook index
- [docs/DESKTOP_AUTONOMY.md](docs/DESKTOP_AUTONOMY.md): reckless desktop autonomy profile, bootstrap, launcher, canaries, and rollback
- [docs/IDENTITY_AND_PERSONALITY.md](docs/IDENTITY_AND_PERSONALITY.md): personality packs, identity stack, and `/personality`
- [docs/MEMORY_AND_GROWTH.md](docs/MEMORY_AND_GROWTH.md): continuity, memory, compaction, and growth surfaces
- [docs/RESEARCH_AND_EXPERIMENTS.md](docs/RESEARCH_AND_EXPERIMENTS.md): research tab, experiments, runners, campaigns, and GPU clouds
- [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md): complete reference for all `thinclaw` terminal commands
- [docs/SURFACES_AND_COMMANDS.md](docs/SURFACES_AND_COMMANDS.md): shared cross-surface vocabulary
- [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md): provider setup and routing
- [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md): native vs WASM channel model
- [docs/SECURITY.md](docs/SECURITY.md): public security and trust overview
- [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md): WASM tools, WASM channels, MCP, registry, and trust boundaries
- [src/setup/README.md](src/setup/README.md): canonical onboarding and setup spec
- [src/tools/README.md](src/tools/README.md): maintainer-facing tool architecture
- [src/workspace/README.md](src/workspace/README.md): workspace and memory model
- [FEATURE_PARITY.md](FEATURE_PARITY.md): parity tracker plus ThinClaw-first feature ledger

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

When behavior changes, update the relevant canonical docs in the same branch. If the change affects a tracked feature, update [FEATURE_PARITY.md](FEATURE_PARITY.md) too.

## License

Licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
