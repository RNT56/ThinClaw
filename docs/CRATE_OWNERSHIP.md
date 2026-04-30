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
| `thinclaw-media` | media content, storage helpers, and channel media limits |
| `thinclaw-workspace` | workspace core, repository helpers, search/chunking, document helpers |
| `thinclaw-db` | persistence traits, DB backends, migrations, DB contract-facing glue |
| `thinclaw-llm-core` | provider traits and transport-neutral LLM DTOs |
| `thinclaw-llm` | provider factory/runtime, routing, usage tracking, provider presets, rig adapter |
| `thinclaw-tools-core` | core tool traits, descriptors, rate limiting, URL guard |
| `thinclaw-tools` | tool registry core, smart approval, browser args, intent display, MCP primitives, WASM tool primitives |
| `thinclaw-channels-core` | core channel traits and message/status types |
| `thinclaw-channels` | channel manager, Gmail/HTTP slices, reactions/status helpers, WASM channel primitives |
| `thinclaw-gateway` | gateway DTOs, auth helpers, SSE/log/static-file primitives |
| `thinclaw-agent` | extracted agent support types, prompt helpers, cost guard, routine records, agent-owned ports |
| `thinclaw-app` | root-independent startup/runtime helper functions |

## Root-Owned Runtime Still In Root

The following areas are intentionally still root-owned until their dependency
cycles are removed through narrow ports/adapters:

- agent loop, dispatcher, sessions, subagents, learning, outcomes, scheduler,
  routine engine, and worker orchestration
- tool execution pipeline, execution backends, root-dependent built-ins, WASM
  wrapper/loader/oauth/storage, MCP client/auth adapters
- native channel transports that depend on root config, media, pairing, TUI,
  platform-specific helpers, or agent submission wiring
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
