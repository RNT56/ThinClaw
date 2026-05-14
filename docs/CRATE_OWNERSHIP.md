# Crate Ownership

ThinClaw is split into focused workspace crates with the root package kept as
the compatibility facade and binary entrypoint.

## Rule Of Thumb

- Internal crates import each other directly as `thinclaw_*`.
- Internal crates must not import the root `thinclaw` package.
- Root `src/*` modules preserve public paths such as `thinclaw::agent`,
  `thinclaw::tools`, `thinclaw::channels`, `thinclaw::db`, and
  `thinclaw::workspace`.
- Root modules should be facades or app wiring unless they own host-only runtime
  behavior that has not yet been ported behind crate-level traits.

## Current Runtime Crates

| Crate | Owns |
|---|---|
| `thinclaw-types` | transport-neutral records, DTOs, small shared enums, and boundary data |
| `thinclaw-safety` | safety primitives that do not depend on LLM/provider runtime |
| `thinclaw-platform` | state paths, shell/platform helpers, host capability detection |
| `thinclaw-settings` | persisted settings structs, defaults, and DB map conversion |
| `thinclaw-config` | resolved config, config formats, provider catalog helpers, env helpers, LLM config records |
| `thinclaw-secrets` | secret types, crypto, memory store, keychain/store backends |
| `thinclaw-context` | context helpers and context-facing data |
| `thinclaw-history` | conversation, outcome, trajectory, and history records |
| `thinclaw-experiments` | experiment records and experiment DTOs |
| `thinclaw-media` | media content, storage helpers, channel media limits, and document text extraction primitives |
| `thinclaw-workspace` | workspace core, repository helpers, search/chunking, document helpers |
| `thinclaw-db` | persistence traits, DB backends, migrations, DB contract-facing glue |
| `thinclaw-llm-core` | provider traits and transport-neutral LLM DTOs |
| `thinclaw-llm` | provider factory/runtime, routing, usage tracking, provider presets, rig adapter |
| `thinclaw-tools-core` | core tool traits, descriptors, rate limiting, URL guard |
| `thinclaw-tools` | tool registry core and root-independent registry composition, smart approval, browser args, intent display, MCP protocol/config/session/client runtime and OAuth helpers, execution DTO/local execution, shell command runtime behind sandbox/ACP/smart-approval ports, execute-code subprocess/tool-RPC runtime behind execution and host-tool ports, background process management, filesystem tools behind host hooks, extension-management tool behavior behind a lifecycle port, desktop-autonomy tool behavior behind a host port, CDP browser automation behind a Docker runtime port, WASM tool primitives/runtime wrapper/loader/watcher, shell-security policy, HTTP/search helpers, root-independent built-ins including messaging adapters, platform/device tools, document extraction, vision analysis, LLM selection/listing, MoA/advisor tools, Nostr social actions, external-memory tool behavior behind a learning port, agent-management and subagent tool behavior behind ports, TTS, and accessibility-browser control |
| `thinclaw-channels-core` | core channel traits and message/status types |
| `thinclaw-channels` | channel manager, native channel transports for Signal, Discord, Gmail, HTTP, BlueBubbles, Apple Mail, iMessage, and Nostr, TUI channel mechanics/DTOs, reactions/status helpers, pairing store support, WASM channel primitives/runtime wrapper/loader/router/watcher |
| `thinclaw-gateway` | gateway DTOs, auth helpers, SSE/log/static-file primitives, status-to-SSE mapping, submission helpers, gateway service ports |
| `thinclaw-agent` | extracted agent support types, session/task domain, session-search rendering/windowing behind a transcript-store port, trajectory record/logging types, agent environment/eval runner framework behind a concrete-agent port, context monitoring and compaction algorithms behind summarizer/archive ports, self-repair policy and repair loop behind context/store/builder ports, run artifact records plus run driver/harness behind runtime lookup and memory-sync ports, filesystem checkpoints, command routing and dispatcher helper logic, workspace-level agent routing and agent registry logic behind persistence/seeding ports, prompt helpers, cost guard, routine records and LLM-facing routine tools behind store/engine/outcome ports, job monitor event forwarding, agent-owned ports |
| `thinclaw-app` | root-independent startup/runtime policy, app assembly DTOs, quiet startup spinner behavior |

## Root-Owned Runtime Still In Root

The following areas are intentionally still root-owned until their dependency
cycles are removed through narrow ports/adapters:

- agent loop, dispatcher, subagents, learning orchestration, trajectory
  hydration adapters, session-search DB adapter, agent-registry DB/workspace
  adapters, self-repair context/DB/builder/registry adapters, compaction
  LLM/safety/workspace adapters, concrete `Agent` env adapter, outcomes,
  scheduler, concrete routine engine, worker orchestration, run artifact
  runtime-descriptor adapters, gateway SSE job-monitor adapters, and root
  adapters for session persistence
- root-dependent tool adapters, app-specific registration, DB-backed MCP
  adapters, sandbox/job orchestration adapters, concrete skill/memory tool
  adapters, root filesystem host hooks for checkpoints/ACP forwarding, root
  execution-backend adapters for shell/process/sandbox compatibility, the root
  `DesktopAutonomyManager` adapter for desktop-autonomy tools, the root
  `ExtensionManager` adapter for extension tools, the root
  `AgentRegistry` adapter for agent-management tools, and the root
  `SubagentExecutor` adapter for subagent tools
- root channel adapters for ACP stdio, REPL, the concrete TUI app runner, HTTP
  config conversion, and gateway route/app-state wiring that still depend on
  root agent, tool, DB, settings, CLI skin, or concrete app services
- `AppBuilder` and full dependency assembly

Do not mark these modules as extracted just because adjacent DTO/runtime helpers
live in crates. The root facades are compatibility boundaries, not proof that
every runtime path has moved.

## Verification

Use these structural checks when changing crate boundaries:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo check --workspace --features full
cargo clippy --workspace --all-targets --features full -- -D warnings
cargo test --workspace --no-run --features full
rg "use thinclaw::" crates
rg "crate::(agent|tools|channels|llm|db|workspace)" crates
```

The final two searches should have no matches except local crate-module paths
that are intentionally owned by the crate being searched.
