# ThinClaw Development Guide

This file is the maintainer-facing map for ThinClaw. It is intentionally high-level and should stay aligned with the current codebase rather than trying to mirror every file or every setup step.

## What ThinClaw Is

ThinClaw is a Rust-based, self-hosted personal agent runtime. It can run:

- as a standalone binary
- as a long-running service with the web gateway
- embedded inside Scrappy

The system combines:

- a multi-session agent runtime
- multiple operator-facing surfaces: CLI, web gateway, and channels
- hybrid extensibility through native Rust, WASM, and MCP
- layered safety controls around tools, secrets, network access, and external content

## Core Design Ideas

- **Control over convenience**: ThinClaw assumes the operator chooses where it runs, which models it uses, and which integrations are trusted.
- **Security as architecture**: safety is split across sandboxing, tool policy, secret injection, network controls, and trust-boundary decisions.
- **Hybrid extensibility**: native Rust is used where persistent connections or local system access matter; WASM is used where hot reload and credential isolation matter; MCP is used for external tool ecosystems.
- **Proactive runtime**: routines, heartbeat, subagents, memory, and the gateway are part of the operating model, not bolt-on features.

## Canonical Docs

Use these as the current documentation authority before updating surrounding docs:

| Topic | Canonical Doc |
|---|---|
| Setup and onboarding | `src/setup/README.md` |
| Boot and runtime flow | `Agent_flow.md` |
| Deployment and remote access | `docs/DEPLOYMENT.md` |
| Channel architecture | `docs/CHANNEL_ARCHITECTURE.md` |
| Extension architecture | `docs/EXTENSION_SYSTEM.md` |
| Tool implementation guidance | `src/tools/README.md` |
| Workspace and memory model | `src/workspace/README.md` |
| Security and network model | `src/NETWORK_SECURITY.md` |
| LLM provider catalog | `src/config/provider_catalog.rs` |
| LLM provider user guide | `docs/LLM_PROVIDERS.md` |
| Feature-tracking changes | `FEATURE_PARITY.md` |

When these docs disagree with broad overview docs, code and canonical docs win.

## Repo Shape

The codebase is easier to reason about by subsystem than by file count.

- `src/agent/`: agent loop, sessions, subagents, routines, cost guard, dispatcher
- `src/channels/`: native channels, gateway, HTTP ingress, WASM channel runtime
- `src/cli/`: operator-facing CLI commands
- `src/config/`: config loading, overlays, defaults, feature-specific settings
- `src/context/`: compaction, memory injection, read audit
- `src/extensions/`: extension lifecycle, registry integration, manifest handling
- `src/llm/`: provider selection, routing, failover, pricing, caching, discovery
- `src/skills/`: skill registry, workspace/bundled skill loading, hot-reload
- `src/safety/`, `src/sandbox/`, `src/secrets/`: trust boundaries and execution controls
- `src/setup/`: onboarding wizard and first-run configuration
- `src/tools/`: built-in tools, extension tools, WASM runtime, MCP client
- `src/workspace/`: persistent memory, search, citations, chunking, repository support

## Runtime Model

At a high level:

1. Bootstrap loads `.env` and config overlays.
2. `AppBuilder` initializes the database, secrets, LLM stack, tools, channels, and extensions.
3. The runtime wires operator surfaces and background systems around the session manager and dispatcher.
4. The agent handles interactive work, background work, and external events through the same core runtime.

For a deeper walkthrough, use `Agent_flow.md`.

## Current Architecture Notes

- The web gateway is the control plane. It is operator-facing infrastructure, not just another chat channel.
- Channel delivery is hybrid. Some channels are native Rust modules; others are packaged WASM channel artifacts.
- Channel-specific formatting/rendering guidance is owned by the channel layer, not prompt assembly. Native channels should override `Channel::formatting_hints()`, and packaged WASM channels should declare `formatting_hints` in their `*.capabilities.json` metadata. Do not add channel-name switches back into `src/llm/reasoning.rs`.
- Extension flows are split. `tool`, `mcp`, and registry installs are related but not interchangeable surfaces.
- The onboarding wizard is richer than older docs imply. Do not restate its steps casually; point readers to `src/setup/README.md` and the wizard code.
- MCP servers are operator-trusted external processes or services, not sandboxed WASM extensions.

## Local Development

```bash
# Formatting
cargo fmt

# Lint
cargo clippy --all --benches --tests --examples --all-features

# Tests
cargo test

# Run locally with logs
RUST_LOG=thinclaw=debug cargo run
```

Useful variants:

```bash
# Air-gapped / embedded WASM build
cargo build --release --features bundled-wasm

# Rebuild packaged WASM artifacts
./scripts/build-all.sh
```

## Documentation Rules

- Keep `README.md` as the front door, not the full manual.
- Keep subsystem docs thinly scoped and explicit about ownership.
- Avoid brittle counts, stale inventories, and “default forever” claims unless the code guarantees them.
- When behavior changes, update the relevant canonical docs in the same branch.
- If the change affects tracked feature behavior, update `FEATURE_PARITY.md` too.

## Common Update Triggers

- If you change onboarding, update `src/setup/README.md` and any user-facing setup references.
- If you change delivery architecture, update `docs/CHANNEL_ARCHITECTURE.md` and the affected channel guides.
- If you change channel formatting behavior, update the owning native channel or WASM channel manifest first, then update `docs/CHANNEL_ARCHITECTURE.md` if the operator-facing behavior changed.
- If you change extension flows, update `docs/EXTENSION_SYSTEM.md`, `src/tools/README.md`, and any affected tool docs.
- If you change security boundaries, update `src/NETWORK_SECURITY.md` and any top-level trust/safety wording.

## Preferred Maintainer Workflow

1. Find the canonical doc for the area.
2. Confirm code truth before editing overview docs.
3. Update code-adjacent spec docs first when behavior changed.
4. Update broader docs second.
5. Check whether `FEATURE_PARITY.md` needs a coordinated status change.
