<p align="center">
  <img src="Thinclaw_IC_01_nobg.png" alt="ThinClaw" width="180"/>
</p>

<h1 align="center">ThinClaw</h1>

<p align="center">
  <em>A self-hosted AI collaborator for real work: private, durable, and biased toward useful output.</em>
</p>

<p align="center">
  <a href="https://github.com/RNT56/ThinClaw/releases"><img src="https://img.shields.io/github/v/release/RNT56/ThinClaw?style=flat-square&color=2ea44f&label=release" alt="Latest Release" /></a>
  &nbsp;
  <a href="https://github.com/RNT56/ThinClaw/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/RNT56/ThinClaw/ci.yml?branch=main&style=flat-square&label=CI" alt="CI" /></a>
  &nbsp;
  <a href="https://github.com/RNT56/ThinClaw/blob/main/LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-0969da?style=flat-square" alt="License" /></a>
</p>

<p align="center">
  <a href="#first-run"><img src="https://img.shields.io/badge/First_Run-2ea44f?style=flat-square" alt="First Run" /></a>&nbsp;
  <a href="#why-thinclaw"><img src="https://img.shields.io/badge/Why_ThinClaw-8250df?style=flat-square" alt="Why ThinClaw" /></a>&nbsp;
  <a href="#personality-that-adapts"><img src="https://img.shields.io/badge/Personality-6f42c1?style=flat-square" alt="Personality" /></a>&nbsp;
  <a href="#what-you-can-use-it-for"><img src="https://img.shields.io/badge/Use_Cases-0969da?style=flat-square" alt="Use Cases" /></a>&nbsp;
  <a href="#install-options"><img src="https://img.shields.io/badge/Install-f59e0b?style=flat-square" alt="Install" /></a>&nbsp;
  <a href="#security-and-trust"><img src="https://img.shields.io/badge/Security-c2410c?style=flat-square" alt="Security" /></a>&nbsp;
  <a href="#documentation-map"><img src="https://img.shields.io/badge/Docs-57606a?style=flat-square" alt="Docs" /></a>&nbsp;
  <a href="#development"><img src="https://img.shields.io/badge/Development-24292f?style=flat-square" alt="Development" /></a>
</p>

---

## What Is ThinClaw?

Your AI assistant should not vanish when the tab closes.

ThinClaw is a practical, privacy-respecting AI collaborator for real work. Clear
head. Steady hands. Useful output. It is warm but not flattering, opinionated
but not reckless, careful with trust, and biased toward getting things actually
done.

ThinClaw is self-hosted, so the agent has a place to live. It keeps a durable
identity, remembers useful context, runs approved tools, connects to channels,
schedules routines, and stays inside policy you control.

Run it on your laptop, a Mac Mini, a Raspberry Pi, a VPS, or inside ThinClaw
Desktop. Talk to the same agent from the terminal, full-screen TUI, web gateway,
chat channels, the desktop app, or background jobs. Same identity. Same memory.
Same rules.

> ThinClaw is for people who want an agent with a home address, not another
> disposable chat thread.

Put it on a machine you trust. Connect the services you actually use. Let it
remember project facts, follow up, watch routine work, and report back through
the surface you already have open. ThinClaw is not a chatbot you revisit. It is
an agent you come back to.

| The Need | ThinClaw's Answer |
|---|---|
| Chat that survives the session | Durable identity, memory, workspace context, and continuity commands |
| Automation that is not scattered | Routines, heartbeat, jobs, channels, notifications, and tools in one runtime |
| AI that can touch real systems carefully | Explicit trust boundaries for secrets, tools, code, network access, and desktop control |
| A private agent stack | Local or self-hosted deployment with your providers, your data paths, and your policies |
| Extensibility without chaos | Native integrations for trusted host work, sandboxed WASM for scoped components, and MCP for external ecosystems |

## First Run

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

Try one useful loop:

```text
Remember that this machine is my personal ThinClaw node.
Summarize what you can do from this environment.
Create a follow-up reminder to review setup tomorrow.
```

The point is not the prompt. The point is that ThinClaw has somewhere to put the
memory, a runtime to keep operating, and surfaces you can return to later.

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

## What You Can Use It For

| Use Case | What ThinClaw Lets You Do |
|---|---|
| Personal operations agent | Keep routines, reminders, logs, service checks, and notifications close to the machine that owns them. |
| Project memory | Give an agent continuity across repos, docs, sessions, and long investigations without starting from zero every time. |
| Adaptable working partner | Shift between balanced, professional, creative, research, mentor, minimal, technical, playful, and custom session modes. |
| Channel-native assistant | Talk to the same agent through the terminal, WebUI, Telegram, Discord, Slack, Gmail, Nostr, and more. |
| Remote command center | Run ThinClaw on a Mac Mini, Raspberry Pi, VPS, or workstation and reach it through a gateway or chat channel. |
| Local media lab | Connect ComfyUI for image generation, workflow health checks, dependency checks, and generated-media artifacts. |
| Controlled desktop autonomy | Opt into host-level UI automation with evidence capture, approval boundaries, rollout, and rollback. |

## Why ThinClaw

### A Useful Character, Not a Novelty

ThinClaw is designed to feel like a capable working presence: clear, discreet,
direct, and protective of your context. It is the collaborator you trust with
the messy middle of real life and real work: private context, unfinished
thoughts, half-built projects, ambiguous requests, and the need to actually
move.

That character is not mascot lore or a nicer prompt wrapper. It is how the agent
makes decisions under ambiguity, how it handles trust, and how it keeps work
moving without becoming pushy or performative. ThinClaw does not perform
helpfulness. It helps.

### One Agent, Many Doors

The terminal is not the product. The WebUI is not the product. Telegram is not
the product. The agent is the product, and ThinClaw gives that agent multiple
doors into the same identity, memory, tools, and policy.

### Memory With a Home

ThinClaw is built around continuity. It can remember workspace context, identity
details, routines, outcomes, and useful notes in places you control. You decide
what is configured, what is persisted, and what gets forgotten.

### Background Work That Keeps Moving

Agents become more useful when they can keep watch, run scheduled work, follow
up, report status, and hand work between surfaces. ThinClaw treats routines,
heartbeat, jobs, notifications, and channels as part of the core runtime.

### Control You Can Audit

ThinClaw is not a remote black box. It runs where you put it. Providers,
extensions, data paths, secret stores, sandbox backends, and desktop autonomy are
explicit configuration choices.

### Built for Real Tools

Useful agents need more than prompts. ThinClaw supports built-in tools, packaged
WASM tools and channels, MCP servers, native integrations, ComfyUI workflows,
subagents, and controlled host automation.

## Personality That Adapts

ThinClaw has a durable character, not a locked costume. Its base identity lives
in a canonical soul: practical, discreet, candid, warm, and outcome-driven. That
identity travels across projects and surfaces so the same agent can meet you in
the terminal, WebUI, chat channels, or background work without becoming a new
stranger every time.

You can then shape how it shows up.

| Layer | What It Does |
|---|---|
| Durable soul | The long-lived core: privacy, trust, helpfulness, continuity, and operating principles |
| Personality pack | The initial flavor chosen during setup, such as balanced, professional, mentor, or flow state |
| `/personality` overlay | A temporary session tone or working mode without rewriting the durable identity |
| Workspace identity files | Project-specific context through `IDENTITY.md`, `USER.md`, `AGENTS.md`, and optional local overlays |

Built-in personality packs include:

| Pack | Best For |
|---|---|
| `balanced` | Grounded everyday collaboration |
| `professional` | Crisp workplace-ready support |
| `creative_partner` | Lateral thinking, drafts, naming, ideation, and creative direction |
| `research_assistant` | Evidence-first synthesis, careful uncertainty, and source-driven work |
| `mentor` | Patient guidance, explanations, and skill-building |
| `minimal` | Terse answers, low ceremony, and quiet competence |
| `flow_state` | Composed intensity, momentum, sharper taste, and receipts |

Session overlays let you shift tone on demand:

```text
/personality concise
/personality technical
/personality playful
/personality eli5
/personality reset
```

ThinClaw can also accept a custom session personality in plain language. Ask for
`/personality skeptical reviewer`, `/personality calm operator`, or a tone that
fits the moment. The overlay changes voice, density, and collaboration style. It
does not relax privacy, consent, permissions, or safety boundaries.

That is the point: ThinClaw can feel personal without becoming loose with trust.
It can be terse in the terminal, careful in research, warm in planning, direct in
code review, or more imaginative when you need creative force, while still
remaining the same agent underneath.

## Why Not Just Hosted Chat Or Scripts?

| If You Need | Hosted Chat | Scripts And Cron | ThinClaw |
|---|---|---|---|
| One durable agent identity | Product-dependent | Manual | Built in |
| Memory across surfaces | Limited | Manual | Built in |
| Local or self-hosted control | No | Yes | Yes |
| Channels, routines, tools, and jobs together | Limited | Fragmented | Built in |
| Explicit trust boundaries | Product-dependent | Manual | Built in |
| A runtime that can grow with you | No | Hard to maintain | Yes |

## Who It Is For

- People who want a personal AI runtime they can run on their own machines.
- Builders who want memory, tools, routines, channels, and policy in one place.
- Operators who care where secrets live and which systems an agent can touch.
- Teams experimenting with self-hosted agent infrastructure instead of another hosted chat wrapper.

## Who It Is Not For

- You only want a zero-configuration hosted chatbot.
- You do not want to connect providers, secrets, channels, or local services.
- You do not want to think about trust boundaries for real tools.
- You need a pure SaaS product with no operator-owned runtime.

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
| ThinClaw Desktop | Desktop companion app embedding ThinClaw as a local or remote runtime | [apps/desktop/README.md](apps/desktop/README.md) |
| Reckless desktop autonomy | Operator-approved host-level desktop automation | [docs/DESKTOP_AUTONOMY.md](docs/DESKTOP_AUTONOMY.md) |

## Core Capabilities

| Area | Capabilities |
|---|---|
| Agent identity | Durable soul, `personality_pack` defaults, `/personality` overlays, custom session tones, workspace identity files, and `/vibe` compatibility |
| Memory and continuity | Workspace-backed memory, search, citations, identity files, `/compress`, and `/summarize` |
| Surfaces | CLI, TUI, web gateway, native channels, WASM channels, background jobs, and ThinClaw Desktop |
| Routines and jobs | Heartbeat, schedules, notifications, job logs, background work, and follow-up surfaces |
| Channels | Telegram, Signal, Discord, Slack, Matrix, Nostr, Gmail, iMessage, BlueBubbles, Apple Mail, voice-call, APNs/browser-push wake paths, and packaged WASM channels |
| Tools and extensions | Built-in tools, WASM tools, WASM channels, MCP servers, registries, and policy boundaries |
| Models | Multi-provider routing, failover, provider setup, model guidance, and cost controls |
| Gateway | Chat, memory, routines, logs, extensions, providers, projects, skills, and settings |
| Media generation | ComfyUI-backed `image_generate`, workflow health checks, dependency checks, and renderable generated-media artifacts |
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
| ThinClaw Desktop | Local or remote ThinClaw runtime in the desktop companion app | [apps/desktop/README.md](apps/desktop/README.md) |

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

## Install Options

GitHub Releases are the normal path. The installer downloads a prebuilt binary,
verifies its SHA256 checksum, and installs `thinclaw` into `~/.local/bin` by
default:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh
```

For Pi, VPS, SD-card, and other small-machine installs, use the edge artifact:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/RNT56/ThinClaw/releases/latest/download/thinclaw-installer.sh | sh -s -- --profile edge
```

Release artifacts publish the regular `full` binary for supported Linux, macOS,
and Windows targets, plus Linux `edge` artifacts for small machines.

Source builds, feature profiles, and maintainer workflows live in
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Documentation Map

| Need | Start Here |
|---|---|
| Audience-first docs index | [docs/README.md](docs/README.md) |
| Deployment decision tree | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| ComfyUI image generation | [docs/COMFYUI_MEDIA_GENERATION.md](docs/COMFYUI_MEDIA_GENERATION.md) |
| Platform runbooks | [docs/deploy/](docs/deploy/) |
| CLI command reference | [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md) |
| LLM provider setup | [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md) |
| Security and trust overview | [docs/SECURITY.md](docs/SECURITY.md) |
| Deep network model | [src/NETWORK_SECURITY.md](src/NETWORK_SECURITY.md) |
| Extensions, WASM, MCP, and registries | [docs/EXTENSION_SYSTEM.md](docs/EXTENSION_SYSTEM.md) |
| Channel architecture | [docs/CHANNEL_ARCHITECTURE.md](docs/CHANNEL_ARCHITECTURE.md) |
| Shared surface commands | [docs/SURFACES_AND_COMMANDS.md](docs/SURFACES_AND_COMMANDS.md) |
| Terminal and WebUI skins | [docs/TERMINAL_SKINS.md](docs/TERMINAL_SKINS.md) |
| Identity and personality | [docs/IDENTITY_AND_PERSONALITY.md](docs/IDENTITY_AND_PERSONALITY.md) |
| Memory and growth surfaces | [docs/MEMORY_AND_GROWTH.md](docs/MEMORY_AND_GROWTH.md) |
| Research and experiments | [docs/RESEARCH_AND_EXPERIMENTS.md](docs/RESEARCH_AND_EXPERIMENTS.md) |
| Contributor workflow | [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) |

## Development

Contributor setup, source builds, local checks, feature profiles, and release
build details live in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). Build-profile
details are tracked in [docs/BUILD_PROFILES.md](docs/BUILD_PROFILES.md).

## License

Licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
