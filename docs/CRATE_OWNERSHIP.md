# Crate Ownership

ThinClaw is split into focused workspace crates with the root package kept as
the compatibility facade and binary entrypoint.

## Root Adapters End State

The crate split targets a root-adapter architecture, not a root-minimal rewrite.
Workspace crates own reusable DTOs, policy, algorithms, port traits, and
root-independent runtimes. Root `src/*` owns public compatibility facades,
`AppBuilder`, binaries, concrete DB/secrets/LLM/tool/channel/gateway wiring,
Docker orchestration, and host side effects.

When moving behavior, keep product behavior stable and cross root/crate
boundaries through narrow ports. Do not move concrete root services into crates
only to reduce root file count.

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
| `thinclaw-types` | transport-neutral records, DTOs, small shared enums, and boundary data, including the routine domain DTOs (`routine` module: `Routine`, `Trigger`, `RoutineAction`, run/event/trigger status + decision enums, guardrails/notify config) consumed by persistence |
| `thinclaw-runtime-contracts` | implementation-free shared runtime DTOs for ThinClaw clients and the Desktop host: asset, direct-runtime, model, provider (incl. `ApiStyle`/`ProviderEndpoint`), runtime, and secret contracts |
| `thinclaw-safety` | safety primitives that do not depend on LLM/provider runtime |
| `thinclaw-platform` | state paths, shell/platform helpers, host capability detection |
| `thinclaw-branding` | shared skin definitions, terminal/web branding art, palette helpers, and skin loading |
| `thinclaw-settings` | persisted settings structs, defaults, and DB map conversion |
| `thinclaw-config` | config formats, provider catalog helpers, env helpers, LLM/config records, webchat presentation config, sandbox/code-agent config records, and pure config resolvers that do not construct root runtime objects |
| `thinclaw-secrets` | secret types, crypto, memory store, keychain/store backends |
| `thinclaw-context` | context helpers and context-facing data |
| `thinclaw-history` | conversation, outcome, trajectory, and history records |
| `thinclaw-identity` | conversation-scope and identity resolution DTOs: conversation kind/scope, resolved identity, linked-conversation recall, actor endpoint references, actor status, and endpoint approval status |
| `thinclaw-soul` | canonical/local soul parsing and rendering, seeded-soul composition, pack name canonicalization, and pack asset markdown |
| `thinclaw-experiments` | experiment records/DTOs, target uniqueness and metadata merge policy, opportunity derivation, lifecycle/status/path policies, lease status/completion policy, subagent planning DTOs, prompt-context helpers, and runner/LLM cost attribution policy |
| `thinclaw-media` | media content, storage helpers, channel media limits, document text extraction primitives, the `MediaExtractor` trait/`MediaExtractError`, and the image/PDF/document extractors. The audio extractor and the `MediaPipeline` glue stay root-owned in `src/media` because they depend on root `crate::config` (would otherwise cycle via `media → config → channels → media`) |
| `thinclaw-workspace` | workspace core, repository helpers, search/chunking, document helpers |
| `thinclaw-db` | persistence traits, DB backends, migrations, DB contract-facing glue |
| `thinclaw-llm-core` | provider traits, transport-neutral LLM DTOs, and root-independent routing policy construction/helpers |
| `thinclaw-llm` | provider factory/runtime, routing, usage tracking, provider presets, rig adapter |
| `thinclaw-tools-core` | core tool traits, descriptors, rate limiting, URL guard, the MCP wire-protocol DTOs (`mcp`), MCP logging-level (`mcp_logging`) and pending-interaction DTOs (`mcp_interaction`), and the runtime-descriptor surface (`execution_descriptor`) — pure data consumed by light crates such as `thinclaw-gateway` |
| `thinclaw-tools` | tool registry core and root-independent registry composition, smart approval, browser args, intent display, MCP config/session/client runtime and OAuth helpers (the pure MCP protocol/logging/interaction DTOs now live in `thinclaw-tools-core` and are re-exported here for path stability), local execution and process backends (the pure runtime-descriptor DTOs/builders now live in `thinclaw-tools-core::execution_descriptor`, re-exported here), shell command runtime behind sandbox/ACP/smart-approval ports, execute-code subprocess/tool-RPC runtime behind execution and host-tool ports, background process management, filesystem tools behind host hooks, extension-management tool behavior and install fallback policy behind a lifecycle port, desktop-autonomy tool behavior behind a host port, CDP browser automation behind a Docker runtime port, WASM tool primitives/runtime wrapper/loader/watcher, shell-security policy, HTTP/search helpers, root-independent built-ins including messaging adapters, platform/device tools, document extraction, vision analysis, LLM selection/listing, MoA/advisor tools, Nostr social actions, external-memory tool behavior behind a learning port, agent-management and subagent tool behavior behind ports, skill publish/check projection policy, TTS, and accessibility-browser control |
| `thinclaw-channels-core` | core channel traits and message/status types, including the `Channel::config_schema()` trait method and the `ConfigSchema`/`ConfigField`/`ConfigOption` DTOs for channel runtime configuration |
| `thinclaw-channels` | channel manager (incl. `config_schema_for`/`config_schemas` accessors and `update_channel_runtime_config`), native channel transports for Signal, Discord, Gmail, HTTP, BlueBubbles, Apple Mail, iMessage, and Nostr (with `config_schema` impls on Signal and Discord), TUI channel mechanics/DTOs, reactions/status helpers, pairing store support, setup-channel validation/projection helpers, ACP wire/status projection helpers, WASM channel primitives/runtime wrapper/loader/router/watcher, root-independent WASM host-config/update policy, and the first-party `ApnsPusher` (`apns_push`) signed-request APNs transport shared by the legacy APNs lifecycle client and the first-party mobile push notifier |
| `thinclaw-gateway` | gateway DTOs, auth helpers, OpenAI-compatible DTO/conversion/validation helpers, chat message-to-turn projection, settings import/redaction/update policies, routine webhook/preview policies, experiment lease-token/limit/status policies, provider credential validation/display/model-selection/routing policies, extension and channel setup-status projection policy, SSE/log/static-file primitives, status-to-SSE mapping, submission helpers, gateway service ports, and device identity (`web::devices`: device registry/store, pairing store, scope-mapping policy, the append-only device-audit log, and the pure content-free push policy `web::devices::push_policy` mapping `SseEvent` → `PushDecision`; `GatewayAuthSource::DeviceToken` and `DeviceContext` in `web::auth`/`web::identity`). Depends on `thinclaw-tools-core` (not `thinclaw-tools`) for MCP/execution DTOs so it stays free of the heavyweight tool runtime (wasmtime/chromiumoxide) |
| `thinclaw-agent` | extracted agent support types, session/task domain, session-search rendering/windowing behind a transcript-store port, trajectory record/logging types, agent environment/eval runner framework behind a concrete-agent port, context monitoring and compaction algorithms behind summarizer/archive ports, self-repair policy and repair loop behind context/store/builder ports, run artifact records plus run driver/harness behind runtime lookup and memory-sync ports, filesystem checkpoints, command routing and dispatcher policy/helper logic, workspace-level agent routing and agent registry logic behind persistence/seeding ports, prompt helpers, cost guard, LLM-facing routine tools behind store/engine/outcome ports, routine trigger-evaluation logic (regex/cron/chrono-tz) and engine trigger/event policy (the pure routine domain DTOs now live in `thinclaw-types::routine`, re-exported from `thinclaw_agent::routine` for path stability so persistence no longer depends on the agent layer), worker loop policy, subagent lifecycle policy, outcome evaluator policy, agent-loop submission/response policy, learning/routine/context ports, job monitor event forwarding, agent-owned ports |
| `thinclaw-app` | root-independent startup/runtime policy, app assembly DTOs, setup/onboarding/profile/provider planning DTOs, bootstrap env planning, quiet startup spinner behavior |
| `thinclaw-repo-projects` | repo-project supervisor domain types and state machines: project/task/run states and transitions, coding backend, merge method, GitHub auth mode, project policy, and merge-gate decision DTOs |
| `thinclaw-portability` | root-independent whole-agent backup bundle format: passphrase AEAD envelope (scrypt + XChaCha20-Poly1305), versioned manifest, and gzip-tar bundle assembly/extraction with path-traversal-safe restore. Gathering the state (DB, workspace) is the root CLI's job (`src/cli/backup.rs`); the crate only builds/opens byte payloads |

## Root-Owned Runtime Still In Root

The following areas are intentionally still root-owned until their dependency
cycles are removed through narrow ports/adapters:

- concrete `Agent` loop/dependency assembly, dispatcher LLM/tool/channel
  execution, subagent Tokio/LLM/tool/channel execution, learning orchestration
  side effects, trajectory hydration adapters, session-search DB adapter,
  agent-registry DB/workspace adapters, self-repair
  context/DB/builder/registry adapters, compaction LLM/safety/workspace
  adapters, concrete `Agent` env adapter, outcome DB/LLM/routing service,
  scheduler side effects, concrete routine engine execution, worker
  orchestration, run artifact runtime-descriptor adapters, gateway SSE
  job-monitor adapters, and root adapters for session persistence
- root-dependent tool adapters, app-specific registration, DB-backed MCP
  adapters, sandbox/job orchestration adapters, concrete skill/memory tool
  adapters and root-owned skill/quarantine scan record mapping, root filesystem host hooks for checkpoints/ACP forwarding, root
  execution-backend adapters for shell/process/sandbox compatibility, the root
  `DesktopAutonomyManager` adapter for desktop-autonomy tools, the root
  `ExtensionManager` adapter for extension tools, the root
  `AgentRegistry` adapter for agent-management tools, and the root
  `SubagentExecutor` adapter for subagent tools
- root channel adapters for ACP stdio, REPL, the concrete TUI app runner, HTTP
  config conversion, WASM host-config conversion from `crate::config::Config`,
  webchat settings loading, and gateway route/app-state wiring that still
  depend on root agent, tool, DB, settings, or concrete app services
- the optional gateway TLS listener (`src/channels/web/tls.rs`, `gateway-tls`
  feature), the device-identity HTTP handlers
  (`src/channels/web/handlers/devices.rs`) and their route wiring in
  `src/channels/web/server.rs`, the first-party mobile push notifier
  (`src/channels/first_party_push.rs`: `FirstPartyPushNotifier` +
  `ApnsPushSender`, spawned from `src/async_main.rs` only when APNs config is
  present), and the `thinclaw devices` CLI
  (`src/cli/devices.rs`) — these depend on root app-state, the concrete
  `DeviceRegistry`/`DeviceStore` wiring, and root gateway config
- root config entrypoints that load dotenv/TOML/DB/settings overlays, construct
  runtime objects or provider instances, or convert into sandbox/tool/WASM
  runtime configs
- setup/onboarding prompts, TUI shell, provider validation, secrets writes,
  filesystem persistence, DB reconnect, settings mutation, and first-run side effects
- bootstrap filesystem writes, environment mutation, legacy migration side
  effects, command execution, and dotenv loading
- `AppBuilder`, binaries, and full dependency assembly

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
rg '^\s*thinclaw\s*=|^\s*\[.*\.thinclaw\]' crates -g Cargo.toml
rg 'package\s*=\s*"thinclaw"' crates -g Cargo.toml
rg 'thinclaw-agent' crates/thinclaw-db/Cargo.toml
```

The structural searches should have no matches. CI runs the root-import,
root-package, and `thinclaw-db`→`thinclaw-agent` dependency guards in the
code-style job so crate-boundary violations fail before the expensive compile
matrix. Persistence (`thinclaw-db`) must not depend on the agent layer.
