<p align="center">
  <img src="Thinclaw_IC_01_nobg.png" alt="ThinClaw" width="200"/>
</p>

<h1 align="center">ThinClaw</h1>

<p align="center">
  <strong>A self-hosted personal agent runtime in Rust</strong>
</p>

<p align="center">
  <a href="https://github.com/RNT56/ThinClaw/releases"><img src="https://img.shields.io/github/v/release/RNT56/ThinClaw?color=green&label=release" alt="Latest Release" /></a>
  <a href="https://github.com/RNT56/ThinClaw/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/RNT56/ThinClaw/ci.yml?branch=main&label=CI" alt="CI" /></a>
  <a href="#quick-start">Quick Start</a>
  <a href="#why-thinclaw">Why ThinClaw</a>
  <a href="#deployment-modes">Deployment Modes</a>
  <a href="#documentation-map">Docs</a>
</p>

---

## What Is ThinClaw?

ThinClaw is a Rust-based agent runtime you run yourself. It can operate as a standalone binary, a long-running service with a web gateway, or as the backend engine embedded inside Scrappy.

It is built around a few core ideas:

- operator-controlled deployment, models, and integrations
- layered safety around secrets, tools, network access, and external content
- hybrid extensibility through native Rust, WASM, and MCP
- a proactive runtime built around channels, routines, memory, and background work

ThinClaw is not just a chat wrapper. It is the runtime that handles sessions, tools, channels, persistence, and policy.

## Quick Start

The fastest local path is:

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

# 3. Start ThinClaw locally
thinclaw run --no-onboard

# 4. Open the gateway
# http://127.0.0.1:3000
```

For a deeper setup path, including service mode, remote access, and provider guidance, use the docs hub at [docs/README.md](docs/README.md).

The onboarding flow now uses a calmer "Humanist Cockpit" framing in both CLI and TUI modes, with shared readiness summaries and saved follow-up notes so operators can pause setup without losing context.

## Why ThinClaw

### 1. Security Is Part of the Architecture

ThinClaw’s safety story is not one toggle. It is split across host-boundary secret injection, sandboxing, tool policy, network controls, and explicit trust boundaries.

- WASM tools and WASM channels are sandboxed and capability-scoped.
- Native channels and built-in tools run in the trusted host runtime.
- MCP servers are operator-trusted external processes or services, not sandboxed plugins.

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
- as a long-running service on macOS or Linux
- behind the built-in gateway
- embedded inside Scrappy

## Core Capabilities

- Multi-surface operation through the CLI, gateway, channels, and background jobs
- Humanist Cockpit onboarding with shared CLI/TUI readiness framing and saved follow-up notes
- Hybrid delivery across native channels and packaged WASM channels
- Workspace-backed memory with search, citations, and identity files
- Extension support through built-in tools, WASM tools, and MCP servers
- Multi-provider LLM routing, failover, and cost controls
- Operator-facing gateway UI for chat, memory, routines, logs, extensions, providers, and settings
- Operator-facing transparency controls for subagent detail levels and Telegram subagent session routing

## Deployment Modes

| Mode | Best For | Main Doc |
|---|---|---|
| Local standalone | personal machine, laptop, workstation | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| Long-running service | Mac Mini, Linux host, VPS | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| Remote gateway access | LAN, Tailscale, controlled remote use | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| Scrappy embedding | desktop app workflow | [Agent_flow.md](Agent_flow.md) |

Code-backed local default: the gateway listens on port `3000` unless you configure otherwise.

## Security And Trust

ThinClaw aims for operator control, but it does not claim that every configured integration is equally isolated.

- local data paths, secrets, and policy enforcement are handled in the trusted host runtime
- WASM components are sandboxed
- MCP servers, tunnels, LLM providers, and external services are real trust boundaries

Use the deep docs before relying on a surface for sensitive workflows:

- [docs/SECURITY.md](docs/SECURITY.md)
- [src/NETWORK_SECURITY.md](src/NETWORK_SECURITY.md)
- [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md)
- [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md)

## Documentation Map

Start here, then go deeper by topic:

- [docs/README.md](docs/README.md): audience-first docs index
- [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md): standalone, service, remote, and gateway deployment
- [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md): provider setup and routing
- [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md): native vs WASM channel model
- [docs/SECURITY.md](docs/SECURITY.md): public security and trust overview
- [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md): WASM tools, WASM channels, MCP, registry, and trust boundaries
- [src/setup/README.md](src/setup/README.md): canonical onboarding and setup spec
- [Agent_flow.md](Agent_flow.md): boot and runtime flow
- [src/tools/README.md](src/tools/README.md): maintainer-facing tool architecture
- [src/workspace/README.md](src/workspace/README.md): workspace and memory model
- [FEATURE_PARITY.md](FEATURE_PARITY.md): engineering parity tracker

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
