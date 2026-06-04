# ThinClaw Desktop Runtime Boundaries

Last updated: 2026-05-15

ThinClaw Desktop intentionally contains two AI systems. They serve different
jobs and must not be collapsed into one architecture without an explicit
migration plan.

The app is sound only if these systems stay clearly named, separately owned,
and joined through narrow contracts.

## System A: Direct AI Workbench

The Direct AI Workbench is the standard local/cloud AI app inside Desktop. It is
for direct user-driven chat, local inference, RAG-assisted answers, voice, and
media generation without autonomous agent behavior.

Primary user surfaces:

- `Chat` mode in `frontend/src/components/chat/views/ChatView.tsx`
- `Imagine` mode in `frontend/src/components/chat/views/ImagineView.tsx`
- model, inference, voice, project, and media settings
- Spotlight quick chat when it uses the standard chat path

Primary backend ownership:

- `backend/src/chat.rs`
- `backend/src/sidecar.rs`
- `backend/src/engine/*`
- `backend/src/inference/*`
- `backend/src/rig_lib/*`
- `backend/src/rag.rs`, `vector_store.rs`, `reranker.rs`, `web_search.rs`
- `backend/src/imagine.rs`, `image_gen.rs`, `images.rs`
- `backend/src/history.rs`, `projects.rs`
- `backend/migrations/*.sql`

Runtime model:

- Chat is initiated through Tauri commands such as `direct_chat_stream`.
- Local inference is provided by one compile-time engine family:
  `llamacpp`, `mlx`, `vllm`, `ollama`, or no local engine for cloud-only builds.
- Engines expose OpenAI-compatible HTTP endpoints to the desktop backend.
- Runtime readiness is reported through `LocalRuntimeSnapshot`. The public
  Tauri command redacts local API keys; backend-only consumers may use the
  private snapshot when they need to call the local endpoint.
- Cloud chat/media providers are selected through Desktop config and keychain
  state, not through ThinClaw agent routing.
- RAG/search/media tools are controlled by direct UI toggles such as web search,
  auto mode, attachments, project context, and image-generation settings.

Persistence:

- Uses the Desktop SQLite schema in `backend/migrations`.
- Stores local app conversations in `conversations` and `messages`.
- Stores uploaded/indexed documents in `documents` and `chunks`.
- Stores uploaded images, generated images, RAG documents, TTS output, and STT
  input in `direct_assets` through `DirectAssetStore`.
- `messages.assets` is the canonical `AssetRef[]` attachment field. Legacy
  `messages.images`, `messages.attached_docs`, and `generated_images` stay
  migration-readable only.
- This history is Direct AI Workbench history, not ThinClaw agent memory.

Security boundary:

- No autonomous shell/file/browser execution just because the user is in direct
  chat.
- Tool use is limited to the direct workbench tools explicitly wired for the
  current turn.
- Direct chat can read user-selected project documents and attached files, but
  it must not inherit ThinClaw workspace authority or agent tool approvals.

## System B: ThinClaw Agent Cockpit

The ThinClaw Agent Cockpit is the Desktop surface for the ThinClaw autonomous
agent runtime. It is comparable to WebUI, CLI, TUI, and chat-channel control
surfaces.

Primary user surfaces:

- `ThinClaw` mode in `frontend/src/components/chat/views/ThinClawView.tsx`
- `frontend/src/components/thinclaw/*`
- ThinClaw management pages for memory, routines, skills, MCP, jobs, autonomy,
  channels, routing, cost, learning, experiments, and pairing

Primary backend ownership:

- `backend/src/thinclaw/*`
- `backend/src/setup/commands.rs` entries prefixed `thinclaw_*`
- root `thinclaw` crate imported as `thinclaw_core` by the Tauri backend
- root ThinClaw APIs under `src/api/*`
- root gateway and channel APIs for remote mode

Runtime model:

- Local mode embeds the root ThinClaw runtime in-process through
  `ThinClawRuntimeState` and `ThinClawRuntimeInner`.
- Remote mode talks to a remote ThinClaw HTTP gateway through
  `RemoteGatewayProxy`.
- The frontend invokes stable `thinclaw_*` Tauri commands in both modes.
- Agent stream events are normalized into `UiEvent` and emitted on
  `thinclaw-event`.
- Unsupported remote behavior must return an explicit unavailable reason.

Persistence:

- Uses the root ThinClaw runtime storage and workspace model.
- ThinClaw conversations, memory, jobs, routines, skills, extensions, cost
  records, and channel state are agent-runtime state.
- Workspace markdown such as `AGENTS.md`, `SOUL.md`, `IDENTITY.md`,
  `USER.md`, `TOOLS.md`, `BOOT.md`, and `HEARTBEAT.md` belongs to this system.

Security boundary:

- ThinClaw agent tools are governed by ThinClaw policy, grant checks, and
  approval flow.
- Raw provider secrets must not be returned in remote mode.
- Host-executing behavior such as autonomy, shell, browser, local filesystem
  writes, and arbitrary skill install must remain gated.

## Shared Infrastructure

These pieces may be shared, but only through explicit adapters:

| Shared piece | Allowed use |
| --- | --- |
| Tauri shell | Hosts both systems and dispatches commands. |
| React app shell | Provides navigation, settings, theming, windows, and layout. |
| Keychain / `SecretStore` | Stores provider credentials. Agent access still requires ThinClaw grants. |
| Local inference engines | Report readiness through `LocalRuntimeSnapshot`; `exposurePolicy=shared_when_enabled` means Direct may use the endpoint immediately and ThinClaw may use it only when the local inference toggle is enabled. |
| Cloud provider catalog | May provide model discovery to both systems if the contract is provider/model metadata only. |
| Runtime contracts | `crates/thinclaw-runtime-contracts` is the Desktop-first DTO source for future WebUI/iOS adoption. |
| Generated bindings | Direct Workbench uses `direct_*` command wrappers. Agent Cockpit uses `thinclaw_*` wrappers and `thinclaw-event`. |
| OS permissions | Camera, mic, screen, filesystem, and accessibility prompts may be shared at the host level, but authority must be checked per system. |

The shared pieces are platform services. They are not proof that Direct AI
Workbench state and ThinClaw agent state are the same product model.

## Non-Shared State

These must stay distinct unless a written migration changes them:

| State | Direct AI Workbench | ThinClaw Agent Cockpit |
| --- | --- | --- |
| Chat history | Desktop SQLite `conversations` / `messages` | ThinClaw runtime conversations / threads |
| Long-term memory | Project/RAG documents, chunks, and `direct_assets` | ThinClaw memory/workspace documents |
| Tool permissions | Direct feature toggles for RAG/search/media | ThinClaw policy, grants, and approvals |
| Personas | Desktop personas for direct chat | ThinClaw identity/workspace markdown and runtime persona |
| Routines/jobs | Not owned by direct chat | ThinClaw routines, jobs, learning, autonomy |
| Remote access | None unless a direct-workbench API is designed | ThinClaw gateway remote mode |
| Event stream | Tauri channels and workbench-specific events | `thinclaw-event` `UiEvent` schema |

## Relationship To WebUI

WebUI is a ThinClaw Agent Cockpit surface. It should be compared to Desktop's
`ThinClaw` mode, not to Desktop's direct `Chat` or `Imagine` modes.

Correct comparison:

- WebUI chat thread behavior vs `thinclaw_send_message`
- WebUI SSE events vs `thinclaw-event`
- WebUI provider/routing/settings APIs vs Desktop ThinClaw management pages
- WebUI memory/routines/skills/MCP/jobs/autonomy APIs vs Desktop `thinclaw_*`
  commands and remote proxy routes

Incorrect comparison:

- WebUI agent chat vs Desktop direct `direct_chat_stream`
- WebUI memory vs Desktop RAG documents
- WebUI image/canvas/agent tools vs Imagine Studio image gallery
- WebUI provider routing vs Direct AI Workbench model selection

If a feature must exist on WebUI, Desktop, and a future iOS app, the default
home is the ThinClaw gateway/root API. Desktop-only direct workbench features
should not be promoted to shared platform features by accident.

## Future iOS App

A future iOS app should not inherit the Desktop Tauri architecture. It should
be a client over explicit network contracts.

Recommended iOS split:

- ThinClaw agent control: use the ThinClaw gateway API plus SSE/WebSocket event
  stream.
- Direct AI Workbench behavior: add a separate mobile/direct-inference API only
  if the product explicitly needs non-agent chat/media on iOS.
- iOS local capabilities: expose camera, microphone, files, notifications, and
  app intents as explicit bridge capabilities with per-action consent.
- No hidden sharing of Desktop SQLite history, Desktop RAG chunks, or Tauri
  command names.

The iOS app may share TypeScript/Swift-generated DTOs, provider metadata, and
event schemas. It should not share Desktop process management, sidecar
orchestration, or local model assumptions.

## Rules For Future Work

1. Label every new feature as `Direct AI Workbench`, `ThinClaw Agent Cockpit`,
   or `Shared Platform Service` before implementation.
2. Add direct-chat features under `chat`, `rig_lib`, `inference`, `sidecar`,
   `history`, `projects`, `rag`, `imagine`, or related direct-workbench modules.
3. Add agent features under `thinclaw/*`, root ThinClaw APIs, or gateway routes.
4. Do not make a direct-workbench command call ThinClaw agent internals unless
   the UX explicitly crosses into agent mode.
5. Do not make a ThinClaw command depend on Desktop-only chat history or RAG
   schema.
6. If both systems need a capability, extract a small shared service contract
   first. Keep ownership of state and permissions separate.
7. Update this document when a boundary changes.

## Current Architectural Debt

These are not necessarily bugs, but they must be understood before further
work continues:

- The command registry mixes direct-workbench commands and `thinclaw_*`
  commands in one Tauri builder. This is acceptable for Tauri, but docs and
  tests must treat them as separate command families.
- Desktop has two chat histories. That is acceptable only while the product
  exposes direct chat and agent chat as different modes.
- The names `startThinClawGateway` and `thinclaw_start_gateway` are legacy
  compatibility names. In local mode they start the embedded agent runtime, not
  necessarily an HTTP gateway.
- llama.cpp startup still lives in `SidecarManager` because it depends on Tauri
  sidecar process handling. It is intentionally bridged into the shared runtime
  snapshot instead of being hidden behind a fake `EngineManager` process.
- ThinClaw local-provider config sync uses the shared runtime snapshot and a
  narrow legacy adapter for the old `(port, token, context, family)` config
  shape. New runtime selection code should consume `LocalRuntimeSnapshot`
  directly instead of adding more sidecar probes.
- The remote gateway route matrix covers ThinClaw Agent Cockpit behavior only.
  It is not a contract for Direct AI Workbench remote use.

## Before Continuing Checklist

Before adding or refactoring Desktop behavior:

- Identify the owning system in the PR/issue notes.
- State which persistence model the feature uses.
- State whether it is available in local mode, remote mode, both, or Desktop-only.
- State which secrets/grants are required.
- State which event stream or command family the frontend consumes.
- Update the relevant doc:
  - `runtime-boundaries.md` for ownership changes.
  - `bridge-contract.md` for ThinClaw IPC/event changes.
  - `remote-gateway-route-matrix.md` for ThinClaw remote changes.
  - `runtime-parity-checklist.md` for ThinClaw parity changes.
