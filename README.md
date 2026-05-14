<p align="center">
  <img src="Thinclaw_IC_01_nobg.png" alt="ThinClaw" width="180"/>
</p>

<h1 align="center">ThinClaw</h1>

<p align="center">
  <em>A self-hosted personal agent runtime with Rust underneath.</em>
</p>

<p align="center">
  <a href="https://github.com/RNT56/ThinClaw/releases"><img src="https://img.shields.io/github/v/release/RNT56/ThinClaw?style=flat-square&color=2ea44f&label=release" alt="Latest Release" /></a>
  &nbsp;
  <a href="https://github.com/RNT56/ThinClaw/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/RNT56/ThinClaw/ci.yml?branch=main&style=flat-square&label=CI" alt="CI" /></a>
  &nbsp;
  <a href="https://github.com/RNT56/ThinClaw/blob/main/LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-0969da?style=flat-square" alt="License" /></a>
</p>

<p align="center">
  <a href="#quick-start"><img src="https://img.shields.io/badge/Quick_Start-2ea44f?style=flat-square" alt="Quick Start" /></a>&nbsp;
  <a href="#why-thinclaw"><img src="https://img.shields.io/badge/Why_ThinClaw-8250df?style=flat-square" alt="Why ThinClaw" /></a>&nbsp;
  <a href="#core-capabilities"><img src="https://img.shields.io/badge/Capabilities-0969da?style=flat-square" alt="Capabilities" /></a>&nbsp;
  <a href="#deployment-modes"><img src="https://img.shields.io/badge/Deployment-f59e0b?style=flat-square" alt="Deployment" /></a>&nbsp;
  <a href="#security-and-trust"><img src="https://img.shields.io/badge/Security-c2410c?style=flat-square" alt="Security" /></a>&nbsp;
  <a href="#documentation-map"><img src="https://img.shields.io/badge/Docs-57606a?style=flat-square" alt="Docs" /></a>&nbsp;
  <a href="#development"><img src="https://img.shields.io/badge/Development-24292f?style=flat-square" alt="Development" /></a>
</p>

---

## What Is ThinClaw?

ThinClaw is a self-hosted personal agent runtime for people who want durable agent identity, memory, tools, channels, routines, and policy under their own control.

It can run as a local CLI, a full-screen TUI, a long-running service behind a web gateway, or the backend engine embedded inside Scrappy. The core runtime is Rust; extension points use native Rust, sandboxed WASM, and operator-trusted MCP depending on the job.

| ThinClaw Gives You | Why It Matters |
|---|---|
| Durable agent identity | The same named agent can operate across CLI, WebUI, channels, sessions, and background work. |
| Operator-owned deployment | You choose where it runs, which providers it uses, and which integrations are trusted. |
| Layered safety boundaries | Secrets, tools, code execution, network access, and external content are treated as separate trust surfaces. |
| Proactive runtime surfaces | Channels, routines, heartbeat, memory, notifications, background jobs, and subagents are first-class runtime pieces. |
| Hybrid extensibility | Native Rust handles trusted host work, WASM handles scoped hot-reloadable components, and MCP connects external ecosystems. |

## Quick Start

### macOS / Linux

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh

thinclaw onboard
thinclaw
```

Open the local gateway after startup:

```text
http://127.0.0.1:3000
```

### Windows PowerShell

Install the latest MSI or portable ZIP from [GitHub Releases](https://github.com/RNT56/ThinClaw/releases), then run:

```powershell
thinclaw onboard
thinclaw
```

### Useful First Commands

| Task | Command |
|---|---|
| Run onboarding | `thinclaw onboard` |
| Start the local runtime | `thinclaw` |
| Start the full-screen runtime | `thinclaw tui` |
| Force full-screen onboarding | `thinclaw onboard --ui tui` |
| Reset local ThinClaw state | `thinclaw reset --yes` |
| Show verbose startup logs | `thinclaw --debug --no-onboard` |
| Show verbose `run` logs | `thinclaw --debug run --no-onboard` |

`thinclaw` and `thinclaw run` share the same quiet startup path by default. For targeted log filtering, `RUST_LOG=...` still takes precedence.

### Remote / SSH Hosts

```bash
thinclaw onboard --profile remote
thinclaw gateway access
thinclaw service install
thinclaw service start
```

Use this path for Raspberry Pi, Mac Mini, VPS, and other SSH-managed hosts. The remote profile ships in the regular `thinclaw` release binary; there is no separate remote artifact.

For Raspberry Pi OS Lite native service installs from a downloaded release binary:

```bash
sudo bash deploy-setup.sh --mode native --binary ./thinclaw
```

See [docs/deploy/raspberry-pi-os-lite.md](docs/deploy/raspberry-pi-os-lite.md) for the full helper download and service setup flow.

## Why ThinClaw

### Operator-Owned Runtime

ThinClaw is built for an operator who wants control over deployment, providers, extensions, channels, data paths, secrets, and host privileges. It is a runtime you run, not a remote chat wrapper you decorate.

### Identity, Memory, and Surfaces

A ThinClaw agent has a durable identity and workspace-backed memory. It can operate through local terminal clients, the gateway UI, native channels, packaged WASM channels, and background work without treating each surface as a separate product.

### Security as Architecture

ThinClaw separates trust zones instead of pretending every integration has the same risk profile. Native integrations run in the trusted host runtime, WASM components are capability-scoped, MCP servers are operator-trusted external processes or services, and privileged desktop autonomy is an explicit opt-in mode.

### Extensible Without One Extension Model for Everything

Persistent host integrations, hot-reloadable components, and external tool ecosystems have different safety and lifecycle needs. ThinClaw uses native Rust, WASM, and MCP where each model fits best.

## Run Modes

| Mode | Best For | Start Here |
|---|---|---|
| Local CLI | Personal local runtime, development, direct terminal use | `thinclaw` |
| Full-screen TUI | Keyboard-first local agent cockpit | `thinclaw tui` |
| Web gateway | Browser-based chat, memory, routines, logs, extensions, providers, and settings | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| Service mode | Long-running host, Mac Mini, VPS, Raspberry Pi, Windows service | [docs/deploy/](docs/deploy/) |
| Native channels | Telegram, Signal, Discord, Slack, Matrix, Nostr, Gmail, iMessage, BlueBubbles, Apple Mail, voice-call, APNs, browser-push | [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md) |
| WASM channels and tools | Packaged, capability-scoped extension components | [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md) |
| ComfyUI media generation | Prompt-to-image, workflow execution, and managed local/cloud ComfyUI setup | [docs/COMFYUI_MEDIA_GENERATION.md](docs/COMFYUI_MEDIA_GENERATION.md) |
| Scrappy backend | Embedding ThinClaw as a local or remote runtime | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| Reckless desktop autonomy | Operator-approved host-level desktop automation | [docs/DESKTOP_AUTONOMY.md](docs/DESKTOP_AUTONOMY.md) |

## Core Capabilities

| Area | Capabilities |
|---|---|
| Runtime surfaces | CLI, TUI, web gateway, native channels, WASM channels, and background jobs |
| Identity | `personality_pack` defaults, `/personality` overlays, durable agent identity, and `/vibe` compatibility |
| Shared commands | `/compress`, `/personality`, `/skills`, `/heartbeat`, `/summarize`, `/rollback` |
| Onboarding | Humanist Cockpit CLI/TUI setup with Quick Setup, Advanced Setup, readiness summaries, and saved follow-up notes |
| Channels | Telegram, Signal, Discord, Slack, Matrix, Nostr, Gmail, iMessage, BlueBubbles, Apple Mail, voice-call, APNs/browser-push wake paths, and packaged WASM channels |
| Memory | Workspace-backed memory, search, citations, identity files, and continuity surfaces |
| Extensions | Built-in tools, WASM tools, WASM channels, MCP servers, registries, and policy boundaries |
| Models | Multi-provider routing, failover, provider setup, and cost controls |
| Gateway | Chat, memory, routines, logs, extensions, providers, projects, skills, and settings |
| Media generation | ComfyUI-backed `image_generate`, workflow health/dependency checks, and renderable generated-media artifacts |
| Subagents | Detail-level transparency controls and Telegram subagent session routing |
| Desktop autonomy | Native app adapters, UI automation, evidence capture, managed code autorollout, and rollback |

## Deployment Modes

| Situation | Best For | Start Here |
|---|---|---|
| macOS | Local use, Mac Mini, launchd service | [docs/deploy/macos.md](docs/deploy/macos.md) |
| Windows | Native install, Windows service, WSL guidance | [docs/deploy/windows.md](docs/deploy/windows.md) |
| Linux | Laptop, workstation, server, VPS | [docs/deploy/linux.md](docs/deploy/linux.md) |
| Raspberry Pi OS Lite 64-bit | Pi 4/5 or ARM64 Pi server | [docs/deploy/raspberry-pi-os-lite.md](docs/deploy/raspberry-pi-os-lite.md) |
| Docker | Compose or container deployment | [docs/deploy/docker.md](docs/deploy/docker.md) |
| Remote access | Gateway, Tailscale, webhook tunnels | [docs/deploy/remote-access.md](docs/deploy/remote-access.md) |
| Reckless desktop autonomy | Host-level desktop control with operator approval | [docs/DESKTOP_AUTONOMY.md](docs/DESKTOP_AUTONOMY.md) |
| Scrappy embedding | Local or remote ThinClaw runtime | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |

The local gateway listens on port `3000` unless configured otherwise. For the full deployment decision tree, use [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

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
| Apple Mail / iMessage native adapters | Supported | Unsupported | Unsupported |
| iMessage via BlueBubbles | Supported | Supported | Supported |

## Extension and Trust Model

| Extension Path | Trust Model | Used For |
|---|---|---|
| Native Rust | Trusted host runtime | Persistent connections, local-system access, and built-in integrations |
| WASM tools | Sandboxed and capability-scoped | Hot-reloadable tool components with credential isolation |
| WASM channels | Sandboxed and capability-scoped | Packaged channel components with explicit host capabilities |
| MCP servers | Operator-trusted external process or service | External tool ecosystems and services managed outside the sandbox |
| ComfyUI sidecar | Operator-trusted local or cloud media runtime | Image generation, workflow execution, model/node lifecycle actions |
| Desktop autonomy | Privileged opt-in profile | Host-level app control, UI automation, evidence capture, rollout, and rollback |

## Security and Trust

ThinClaw aims for operator control, but it does not claim every configured integration is equally isolated.

- Local data paths, secrets, and policy enforcement live in the trusted host runtime.
- WASM components are sandboxed and capability-scoped.
- MCP servers, ComfyUI sidecars, tunnels, LLM providers, and external services are real trust boundaries.
- Restricted workspace modes disable unsupported execution paths instead of implying isolation that is not present.
- Docker remains the portable hard-isolation path for code execution; host-local isolation reports its actual backend and capabilities.
- Tool outputs and job surfaces expose runtime backend, runtime family, runtime mode, capabilities, and network-isolation metadata.
- `desktop_autonomy.profile = "reckless_desktop"` adds host-level app, UI, and screen control plus managed code promotion and rollback.

Read the deep docs before relying on a surface for sensitive workflows:

- [docs/SECURITY.md](docs/SECURITY.md)
- [docs/DESKTOP_AUTONOMY.md](docs/DESKTOP_AUTONOMY.md)
- [src/NETWORK_SECURITY.md](src/NETWORK_SECURITY.md)
- [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md)
- [docs/COMFYUI_MEDIA_GENERATION.md](docs/COMFYUI_MEDIA_GENERATION.md)
- [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md)

## Install And Source Builds

GitHub Releases are the normal user path. The installer downloads a prebuilt
binary, verifies its SHA256 checksum, and installs `thinclaw` into
`~/.local/bin` by default:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh
```

For Pi, VPS, SD-card, and other small-machine installs, use the edge artifact:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh -s -- --profile edge
```

Build from source only when you need local code changes. Use `--release` for
source builds you intend to run or install; plain `cargo build` is a
development/debug build and keeps large debug and incremental compiler artifacts
under `target/debug`.

| Profile | Command | What It Adds |
|---|---|---|
| **edge** | `cargo build --release --no-default-features --features edge --bin thinclaw` | libSQL-only small-machine runtime |
| **light** (default) | `cargo build --release --bin thinclaw` | PostgreSQL, libSQL, local gateway, HTML-to-Markdown, doc extraction, timezones |
| **full** | `cargo build --release --features full --bin thinclaw` | ACP, REPL/TUI, tunnel, Docker sandbox, browser, Nostr |
| **desktop** | `cargo build --release --features desktop --bin thinclaw` | libSQL, HTML-to-Markdown, doc extraction, REPL, timezones |

A cold `full` source build should have roughly 15-20 GiB free so Cargo has room
for release dependencies, linker scratch space, and the final binary. The
installed binary is much smaller than the build tree; after copying or installing
`target/release/thinclaw`, reclaim build artifacts with `cargo clean --release`.

Additional opt-in flags not included in `full`: `voice`, `bedrock`, `bundled-wasm`.

For a low-disk source install, keep Cargo artifacts in a temporary target dir:

```bash
tmp="$(mktemp -d)"
cargo install --path . --locked --features full --bin thinclaw --target-dir "$tmp"
rm -rf "$tmp"
```

Release artifacts publish the regular `full` binary for supported Linux, macOS,
and Windows targets, plus Linux `edge` artifacts for small machines.

Full details, custom combinations, and CI matrix: [docs/BUILD_PROFILES.md](docs/BUILD_PROFILES.md)

## Repository Layout

| Path | Purpose |
|---|---|
| [src/](src/) | Core runtime, CLI, gateway, channels, tools, memory, policy, and platform integration |
| [crates/](crates/) | Workspace crates that own extracted subsystem traits, DTOs, and runtime helpers |
| [docs/](docs/) | Canonical user, operator, architecture, security, and deployment docs |
| [deploy/](deploy/) | Linux, Docker, Raspberry Pi, and service helper assets |
| [channels-src/](channels-src/) | Source crates for packaged channel integrations |
| [tools-src/](tools-src/) | Source crates for packaged tool integrations |
| [channels-docs/](channels-docs/) | Channel setup and operation docs |
| [tools-docs/](tools-docs/) | Tool setup and operation docs |
| [patches/](patches/) | Vendored or patched dependency material |

## Documentation Map

| Need | Start Here |
|---|---|
| Audience-first docs index | [docs/README.md](docs/README.md) |
| Deployment decision tree | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| ComfyUI image generation | [docs/COMFYUI_MEDIA_GENERATION.md](docs/COMFYUI_MEDIA_GENERATION.md) |
| Platform runbooks | [docs/deploy/](docs/deploy/) |
| CLI command reference | [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md) |
| Build profiles and feature flags | [docs/BUILD_PROFILES.md](docs/BUILD_PROFILES.md) |
| LLM provider setup | [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md) |
| Security and trust overview | [docs/SECURITY.md](docs/SECURITY.md) |
| Deep network model | [src/NETWORK_SECURITY.md](src/NETWORK_SECURITY.md) |
| Extensions, WASM, MCP, and registries | [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md) |
| Channel architecture | [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md) |
| Crate ownership and thin-shell boundaries | [docs/CRATE_OWNERSHIP.md](docs/CRATE_OWNERSHIP.md) |
| Shared surface commands | [docs/SURFACES_AND_COMMANDS.md](docs/SURFACES_AND_COMMANDS.md) |
| Terminal and WebUI skins | [docs/TERMINAL_SKINS.md](docs/TERMINAL_SKINS.md) |
| Identity and personality | [docs/IDENTITY_AND_PERSONALITY.md](docs/IDENTITY_AND_PERSONALITY.md) |
| Memory and growth surfaces | [docs/MEMORY_AND_GROWTH.md](docs/MEMORY_AND_GROWTH.md) |
| Research and experiments | [docs/RESEARCH_AND_EXPERIMENTS.md](docs/RESEARCH_AND_EXPERIMENTS.md) |
| Onboarding and setup behavior | [src/setup/README.md](src/setup/README.md) |
| Tool implementation guidance | [src/tools/README.md](src/tools/README.md) |
| Workspace and memory model | [src/workspace/README.md](src/workspace/README.md) |
| Feature parity tracking | [FEATURE_PARITY.md](FEATURE_PARITY.md) |
| Contribution guidance | [CONTRIBUTING.md](CONTRIBUTING.md) |

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
