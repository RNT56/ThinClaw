# ThinClaw Development Guide

This file is the maintainer-facing map for ThinClaw. It is intentionally high-level and should stay aligned with the current codebase rather than trying to mirror every file or every setup step.

## What ThinClaw Is

ThinClaw is a Rust-based, self-hosted personal agent. It can run:

- as a standalone binary
- as a long-running service with the web gateway
- embedded inside Scrappy

The system combines:

- a multi-session agent runtime
- a named cross-surface identity with personality packs and temporary session overlays
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
| Identity packs and session personality | `docs/IDENTITY_AND_PERSONALITY.md` |
| Memory, continuity, and growth vocabulary | `docs/MEMORY_AND_GROWTH.md` |
| Research, experiments, and remote runners | `docs/RESEARCH_AND_EXPERIMENTS.md` |
| Terminal commands (`thinclaw run`, etc.) | `docs/CLI_REFERENCE.md` |
| Shared surface command vocabulary | `docs/SURFACES_AND_COMMANDS.md` |
| Setup and onboarding | `src/setup/README.md` |
| Deployment and remote access | `docs/DEPLOYMENT.md` |
| Channel architecture | `docs/CHANNEL_ARCHITECTURE.md` |
| Extension architecture | `docs/EXTENSION_SYSTEM.md` |
| Crate ownership and thin-shell boundaries | `docs/CRATE_OWNERSHIP.md` |
| Tool implementation guidance | `src/tools/README.md` |
| Workspace and memory model | `src/workspace/README.md` |
| Security and network model | `src/NETWORK_SECURITY.md` |
| LLM provider catalog | `src/config/provider_catalog.rs` |
| LLM provider user guide | `docs/LLM_PROVIDERS.md` |
| Build profiles and feature flags | `docs/BUILD_PROFILES.md` |
| Feature-tracking changes | `FEATURE_PARITY.md` |

When these docs disagree with broad overview docs, code and canonical docs win.

## Repo Shape

The codebase is easier to reason about by subsystem than by file count.

- `crates/thinclaw-*`: canonical crate-owned traits, DTOs, runtime helpers, and extracted subsystem pieces
- `src/lib.rs` and root `src/<module>/mod.rs`: compatibility facades for `thinclaw::...` imports
- `src/app.rs`, `src/main.rs`, `src/bin/`: root package entrypoints and host app wiring
- `src/agent/`: remaining root-owned agent loop, dispatcher, subagents, learning, routine engine, scheduler, worker orchestration, and app/runtime adapters
- `src/channels/`: native channel adapters/transports, gateway route wiring, and root WASM channel adapters
- `src/cli/`: operator-facing CLI commands
- `src/config/`: compatibility facades plus root-specific config entrypoints
- `src/context/`: compatibility facades plus root-specific context wiring
- `src/extensions/`: extension lifecycle, registry integration, manifest handling
- `src/llm/`: compatibility facades plus reasoning and root-specific provider wiring
- `src/skills/`: skill registry, workspace/bundled skill loading, hot-reload
- `src/safety/`, `src/sandbox/`, `src/secrets/`: trust boundaries and execution controls
- `src/setup/`: onboarding wizard and first-run configuration
- `src/tools/`: root-dependent built-ins, app-specific registration, sandbox/job adapters, and DB-backed MCP/tool orchestration
- `src/workspace/`: compatibility facades plus root-specific workspace adapters

See `docs/CRATE_OWNERSHIP.md` for the current crate split, dependency-direction
rules, and root-owned runtime areas that still require port/adaptor work.

## Architecture Hygiene

God files are architectural debt, not a neutral style choice. New code should keep modules focused around one domain concept, lifecycle phase, or integration boundary, with a clear reason for each file to change.

- Prefer directory modules when a subsystem grows beyond a single focused file. The `mod.rs` file should stay a façade: declare submodules, re-export the stable public API, and keep only narrowly shared imports or glue.
- Split large subsystems by responsibility: `types`, `core`/manager, orchestration phases, provider adapters, persistence/query code, platform-specific helpers, and test support.
- Add new behavior to the narrowest existing submodule that owns it. Do not grow façade modules, coordinators, or catch-all helper files just because they are convenient.
- Treat repeated unrelated edits to the same file as a signal to extract a cohesive submodule before adding more behavior.
- Preserve public paths during decompositions with `pub use` re-exports. Keep internal cross-module visibility at `pub(super)` or `pub(in crate::...)`; do not widen APIs just to make a split compile.
- Keep tests and fixtures close to the module they validate. Use `tests.rs` or `test_support.rs` for broad behavioral coverage and shared scripted fixtures, not as a dumping ground.
- Avoid vague buckets like `misc`, `common`, or `utils` unless the contents are genuinely cross-cutting and small. Prefer names that describe the owning domain.
- If a coordinator or test file must remain large temporarily, leave the boundaries obvious and extract the next cohesive phase as soon as it stabilizes.

When reviewing or generating changes, block new god-file growth early. A PR that adds substantial unrelated behavior to an already broad file should either split the module first or explain why the code cannot yet be separated safely.

## Runtime Model

At a high level:

1. Bootstrap loads `.env` and config overlays.
2. `AppBuilder` initializes the database, secrets, LLM stack, tools, channels, and extensions.
3. The runtime wires operator surfaces and background systems around the session manager and dispatcher.
4. The agent handles interactive work, background work, and external events through the same core runtime.

For a deeper walkthrough of startup and workspace shaping, use `src/setup/README.md`, `src/workspace/README.md`, and the agent/runtime modules directly.

## Current Architecture Notes

- The web gateway is the control plane. It is operator-facing infrastructure, not just another chat channel.
- The root package is now a compatibility facade plus binary/app wiring for many subsystems. New internal code should import extracted crates directly as `thinclaw_*`, not through root `thinclaw`.
- `thinclaw-tools` owns tool registry core, root-independent registry composition, MCP protocol/config/session/client runtime, MCP OAuth helpers, root-independent execution DTO/local execution, shell command runtime behind sandbox/ACP/smart-approval ports, execute-code subprocess/tool-RPC runtime behind execution and host-tool ports, background process management, extension-management tool behavior, filesystem tools, desktop-autonomy tool behavior behind a port, CDP browser automation behind a Docker runtime port, WASM tool primitives/runtime wrapper/loader/watcher, shell-security policy, HTTP/search helpers, and root-independent built-ins such as time/todo, canvas, device info, Home Assistant, location, camera/screen capture, TTS, document extraction, vision analysis, LLM selection/listing, MoA/advisor consultation, Nostr social actions, external-memory tools, agent-management and subagent behavior behind ports, accessibility-browser control, send-message, and native messaging action adapters. Root `src/tools` still owns app-specific registration, root-dependent adapters, DB-backed MCP config adapters, concrete skill/memory tools, sandbox/job orchestration adapters, and the concrete `DesktopAutonomyManager`, `ExtensionManager`, `LearningOrchestrator`, `AgentRegistry`, `SubagentExecutor`, filesystem host, Docker browser runtime, and shell/process/execute-code execution-backend adapters for those port-backed tools.
- `thinclaw-channels` owns shared channel manager/runtime helpers, pairing store support, native transports for Signal, Discord, Gmail, HTTP, BlueBubbles, Apple Mail, iMessage, and Nostr, TUI channel mechanics/DTOs, plus WASM channel runtime wrapper/loader/router/watcher. Root `src/channels` still owns ACP stdio, REPL, the concrete TUI app runner, HTTP config conversion, and gateway app-state adapters that depend on root services.
- `thinclaw-agent` owns support types, context monitoring, context compaction algorithms behind summarizer/archive ports, self-repair policy and repair loop behind context/store/builder ports, message command routing, dispatcher helper logic, workspace-level agent routing and agent registry logic behind persistence/seeding ports, session/task domain types, session-search rendering/windowing behind a transcript-store port, trajectory record/logging types, agent environment/eval runner framework behind a concrete-agent port, run artifact records plus run driver/harness behavior behind runtime lookup and memory-sync ports, filesystem checkpoint support, routine records and routine tools behind store/engine/outcome ports, job monitor event forwarding, and agent-owned port traits. The full agent loop/dispatcher/runtime remains root-owned until DB, tool, hook, skill, LLM, and channel dependencies are injected through ports.
- `thinclaw-app` owns root-independent startup/runtime policy including quiet startup spinner behavior. Root `src/app.rs` still owns concrete dependency assembly.
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

Temporary dev note:

- If Docker Desktop stops answering during local Postgres work, check host disk pressure first with `df -h /System/Volumes/Data`. On this machine, Docker failed to start because the host disk was nearly full and Docker backend logging hit `no space left on device`.
- The fastest recovery so far was: remove large local build artifacts first (especially repo `target*` directories), then fully restart Docker Desktop by killing stale `com.docker.backend` / `com.docker.virtualization` processes and relaunching the app.
- After Docker comes back, verify with `docker version`, `docker ps`, and then rerun the Postgres-targeted test commands instead of assuming the daemon is healthy.
- A fresh local `pgvector/pgvector:pg17` container is enough for `db_contract` and `schema_divergence`, but broader Postgres-backed integration tests like `workspace_integration` also need the repo migrations applied first. Mirror CI by applying `migrations/V*.sql` into `thinclaw_test` before treating workspace/search failures as a product bug.

## Documentation Rules

- Keep `README.md` as the front door, not the full manual.
- Keep subsystem docs thinly scoped and explicit about ownership.
- Avoid brittle counts, stale inventories, and “default forever” claims unless the code guarantees them.
- When behavior changes, update the relevant canonical docs in the same branch.
- If the change affects tracked feature behavior, update `FEATURE_PARITY.md` too.

## Common Update Triggers

- If you change onboarding, update `src/setup/README.md` and any user-facing setup references.
- If you change identity packs, `/personality`, memory/growth surfaces, or cross-surface vocabulary, update `docs/IDENTITY_AND_PERSONALITY.md`, `docs/MEMORY_AND_GROWTH.md`, and `docs/SURFACES_AND_COMMANDS.md`.
- If you change experiments, research projects, runners, or GPU cloud flows, update `docs/RESEARCH_AND_EXPERIMENTS.md`.
- If you change delivery architecture, update `docs/CHANNEL_ARCHITECTURE.md` and the affected channel guides.
- If you change channel formatting behavior, update the owning native channel or WASM channel manifest first, then update `docs/CHANNEL_ARCHITECTURE.md` if the operator-facing behavior changed.
- If you change extension flows, update `docs/EXTENSION_SYSTEM.md`, `src/tools/README.md`, and any affected tool docs.
- If you change crate boundaries, update `docs/CRATE_OWNERSHIP.md` and keep this file's repo-shape notes aligned.
- If you change security boundaries, update `src/NETWORK_SECURITY.md` and any top-level trust/safety wording.

## Preferred Maintainer Workflow

1. Find the canonical doc for the area.
2. Confirm code truth before editing overview docs.
3. Update code-adjacent spec docs first when behavior changed.
4. Update broader docs second.
5. Check whether `FEATURE_PARITY.md` needs a coordinated status change.
