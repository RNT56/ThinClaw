# ThinClaw Tauri App — Integration Specification

> **Date:** 2026-02-27 (final 2026-03-02) · **Base:** IronClaw v0.12.0 · **Target:** Tauri v2 desktop app (Scrappy)
> **Architecture:** Hybrid API — spawn-and-return for agent turns, direct for queries
> **Approach:** IronClaw as library crate, refactored to expose public API surface
> **Companion doc:** `documentation/latest/ironclaw_library_roadmap.md` — IronClaw library-side roadmap

---

## Implementation Progress

> Last updated: 2026-03-02 08:00 CET — **All 15 phases complete. Zero warnings, zero errors.** Post-integration work (InferenceRouter, Cloud Model Discovery, Cloud Storage A1–A3) also complete.

| Phase | Status | Key Outcome |
|---|---|---|
| **Phase 1**: IronClaw Library Prep | ✅ Complete | `ironclaw::api` module (8 submodules, 1,320 LOC), public Agent methods, `BackgroundTasksHandle`, `desktop` feature flag, `StatusUpdate::Error` variant. Commits: `99d40c8`, `1db954c` |
| **Phase 2**: Scrappy Integration Scaffold | ✅ Complete | `ironclaw_bridge.rs`, `ironclaw_channel.rs`, `ironclaw_types.rs`, `sanitizer.rs`, `ui_types.rs` — all created and wired into `lib.rs`. App boots with `[main] IronClaw engine initialized successfully.` |
| **Phase 2.5a**: libsql/sqlx Conflict | ✅ Resolved | Patched `libsql-0.6.0` via `[patch.crates-io]` — `sqlite3_config()` assertion replaced with `sqlite3_threadsafe()` check. Both databases now coexist. |
| **Phase 2.5b**: SecretsStore Adapter | ✅ Complete | `ironclaw_secrets.rs` bridges Scrappy's macOS Keychain to `ironclaw::secrets::SecretsStore` trait. Wired into `ironclaw_bridge.rs` via `AppBuilder::with_secrets_store()`. |
| **Phase 3**: Command Migration | ✅ Complete | All 66 Tauri commands rewritten from WS RPC → direct IronClaw API calls across 6 batches (chat, sessions, memory, gateway, skills/config, keys). Zero `ws_rpc()` calls remain. |
| **Phase 4**: Dead Code Cleanup | ✅ Complete | Deleted `frames.rs` (155), `normalizer.rs` (838), `ws_client.rs` (748), `ipc.rs` (425) — ~2,166 LOC. Stripped `OpenClawManager` from 350→65 LOC. Zero project warnings. |
| **Phase 5**: Verification | ✅ Complete | `cargo tauri dev` smoke test passed — app boots and runs. Zero build errors, zero project warnings. |
| **Phase 6**: Stub Wiring | ✅ Complete | `openclaw_logs_tail` → `LogBroadcaster::recent_entries()`, `openclaw_cron_run` → `RoutineEngine::fire_manual()`. Added `BackgroundTasksHandle::routine_engine()` accessor in IronClaw. |
| **Phase 7**: Frontend Compat | ✅ Complete | `start_gateway`/`stop_gateway` now implement **real start/stop** of IronClaw agent (was no-op, now full lifecycle). `IronClawState` refactored to `RwLock<Option<IronClawInner>>`. All 25+ command callsites migrated to async `agent()` accessor. `gateway_running` reflects actual engine state. |
| **Phase 8**: Sidecar Cleanup | ✅ Complete | Deleted `openclaw-engine/node_modules/` (625 MB), `main.js`, `package.json`, `package-lock.json`. Kept `deploy-remote.sh`, `Dockerfile` for remote deploy. `tokio-tungstenite` retained (used by fleet.rs). |
| **Phase 9**: Doc & Script Cleanup | ✅ Complete | Removed all `openclaw-engine` refs from `setup.md`, `package.json`, `generate_tauri_overrides.sh`. Updated `TODO.md` with IronClaw integration section. |
| **Phase 10**: Auth-Profiles Cleanup | ✅ Complete | Removed ~292 LOC of dead `auth-profiles.json`/`agent.json`/`models.json` generation from `write_config()` in `engine.rs`. These were consumed by the deleted Node.js engine; IronClaw uses `SecretsStore`. Updated stale comments in `keys.rs` (6 locations) and `secret_store.rs`. |
| **Phase 11**: Patch Warnings | ✅ Complete | Fixed 5 `mismatched_lifetime_syntaxes` warnings in `libsql-0.6.0` patch by adding `<'_>` to `Column` return types across `statement.rs`, `local/statement.rs`, `local/impls.rs`, `replication/connection.rs`. Build is now **fully warning-free**. |
| **Phase 12**: InferenceRouter | ✅ Complete | 29-file `inference/` module: 5 backend traits (Chat, Embedding, TTS, STT, Diffusion), `InferenceRouter` as Tauri state, 14-provider endpoint registry, 24 backend implementations (local + cloud). 2 Tauri commands (`get_inference_backends`, `update_inference_backend`). `UserConfig` extended with 6 per-modality fields. Chat backends fully wired to `UnifiedProvider`. `resolve_provider()` refactored: removed `OpenClawManager` dependency, uses `PROVIDER_ENDPOINTS` for all cloud providers. Router `reconfigure()` eagerly constructs cloud backends from API keys. |
| **Phase 13**: RAG + UI Integration | ✅ Complete | **RAG embedding:** `ingest_document()` + `retrieve_context_internal()` now prioritize `InferenceRouter` embedding backend, bypassing sidecar for cloud users. All 4 call sites updated. **Model library:** Added 15 cloud model entries (Mistral, xAI, Together, Venice, Cohere, Moonshot, MiniMax, NVIDIA, Xiaomi). **SecretsTab:** Added Cohere + Voyage AI API key cards. **Dimension guard:** `reconfigure()` returns `ReconfigureResult` with old/new dims, logs ⚠️ on mismatch. All unknown-backend arms fixed to not early-return. |
| **Phase 14**: Inference Mode UI | ✅ Complete | **InferenceModeTab.tsx:** New settings page (5 modalities, backend switcher, Local/Cloud badges). Lazy-loaded via `Suspense`. **Sidebar:** `'inference-mode'` page + `Sparkles` icon. **ModelSelector:** Local/Cloud badge in chat button (`Cloud`/`Monitor` icons), extended 5→14 provider detection via `resolveProvider`+`hasKeyForProvider` helpers. **ImagineGeneration:** Local/Cloud badges on provider buttons + label. **ModelBrowser:** `isCloudConfigured`/`hasAnyCloud`/badges/`Select Brain` all extended to 14 providers via unified `providerMap`. Unified model list sorting: local first, cloud grouped at bottom. **CloudBrainConfigModal:** Verified 15-provider `PROVIDER_MODELS` + `PROVIDER_DISPLAY_NAMES` already up-to-date. |
| **Phase 15**: Cross-Modal Consistency | ✅ Complete | **SecretsTab:** Added 4 new provider cards (Deepgram STT, ElevenLabs TTS, Stability AI diffusion, fal.ai diffusion) in new "Speech & Image Generation" section. **Keychain:** 6 new slugs (cohere, voyage, deepgram, elevenlabs, stability, fal). **SecretStore:** 15 convenience accessors for all extended providers. **OpenClawConfig/Identity:** key+granted fields for 6 new providers through full lifecycle (constructor, toggle, update, get, save, zeroize Drop). **OpenClawStatus:** 12 new has_*/granted fields wired through gateway.rs. **engine.rs:** `is_provider_granted` extended with 6 new providers. **UserConfig:** Already had all per-modality backend fields (chat, embedding, tts, stt, diffusion + inference_models). |

### Completion Summary

| Metric | Value |
|---|---|
| Total phases | 15 (all ✅) |
| Dead code removed | ~2,458 LOC |
| Commands migrated | 66 (WS RPC → direct API) |
| Build warnings | **0** (including libsql patch) |
| Build errors | **0** |
| Disk space recovered | ~625 MB (`node_modules`) |
| Remaining stub | `openclaw_install_skill_deps` (not called by frontend) |
| Boot sequence | Built-in via `Workspace::system_prompt_for_context()` |
| Inference URL | Handled by IronClaw's own `Config::from_env()` |

### Files Created/Modified (Phase 1 — IronClaw side)

| File | Status | LOC | Purpose |
|---|---|---|---|
| `src/api/mod.rs` | ✅ Modified | 32 | Module declarations + re-exports for all 8 API submodules |
| `src/api/error.rs` | ✅ Created | 55 | `ApiError` enum with `error_code()`, `ApiResult<T>` alias |
| `src/api/chat.rs` | ✅ Created | 215 | `send_message`, `resolve_approval`, `abort` (spawn-and-return pattern) |
| `src/api/sessions.rs` | ✅ Created | 326 | Thread CRUD, paginated history, DB-first with in-memory fallback |
| `src/api/memory.rs` | ✅ Created | 116 | Workspace file CRUD, tree, vector search |
| `src/api/config.rs` | ✅ Created | 99 | Settings CRUD + bulk import/export |
| `src/api/extensions.rs` | ✅ Created | 151 | Extension lifecycle with auto-auth retry |
| `src/api/skills.rs` | ✅ Created | 99 | `RwLock<SkillRegistry>` + `SkillCatalog` operations |
| `src/api/routines.rs` | ✅ Created | 152 | Routine CRUD + `fire_manual` trigger |
| `src/api/system.rs` | ✅ Created | 73 | Engine status snapshot, model info |
| `src/agent/agent_loop.rs` | ✅ Modified | +138 | Public `channels()`, `session_manager()`, `handle_message_external()`, `inject_context()`, `cancel_turn()` |
| `src/channels/channel.rs` | ✅ Modified | +8 | Added `StatusUpdate::Error { message, code }` variant |
| `src/app.rs` | ✅ Modified | +36 | `AppBuilder::with_secrets_store()`, `AppBuilderFlags { no_db }` |
| `Cargo.toml` | ✅ Modified | +19 | `desktop`, `web-gateway`, `full` feature flags |
| `src/lib.rs` | ✅ Modified | +17 | Feature-gated modules, `pub mod api` |

### Files Created/Modified (Phase 2 — Scrappy side)

| File | Status | LOC | Purpose |
|---|---|---|---|
| `backend/src/openclaw/ironclaw_bridge.rs` | ✅ Created | 184 | `IronClawState` lifecycle — init, config, Agent construction, shutdown |
| `backend/src/openclaw/ironclaw_channel.rs` | ✅ Created | 115 | `TauriChannel` implementing `ironclaw::channels::Channel` trait |
| `backend/src/openclaw/ironclaw_types.rs` | ✅ Created | 134 | `StatusUpdate` → `UiEvent` conversion for all 11 variants |
| `backend/src/openclaw/sanitizer.rs` | ✅ Created | — | LLM token stripping (extracted from normalizer) |
| `backend/src/openclaw/ui_types.rs` | ✅ Created | — | `UiEvent` enum + `UiSession`, `UiMessage`, `UiUsage` (extracted from normalizer) |
| `backend/src/openclaw/mod.rs` | ✅ Updated | — | Module declarations for new files |
| `backend/Cargo.toml` | ✅ Updated | — | `ironclaw` dep with `desktop` feature + `[patch.crates-io]` for libsql |
| `backend/src/lib.rs` | ✅ Updated | — | Async IronClaw spawn in setup + graceful shutdown in `RunEvent::Exit` |
| `patches/libsql-0.6.0/` | ✅ Created | — | Local patched copy of libsql with sqlite3_config fix |

### SQLite Threading Conflict — Resolved

Both `libsql` (IronClaw) and `sqlx-sqlite` (Scrappy) call `sqlite3_config()` globally.
Only the first call succeeds — the second gets `SQLITE_MISUSE`. Stock `libsql` panics
on this. **Solution:** patched `libsql-0.6.0/src/local/database.rs:209` to check
`sqlite3_threadsafe()` instead of asserting. Applied via `[patch.crates-io]` in
`backend/Cargo.toml`.

### Bugs Fixed (from Phase 3 Review)

1. **`ironclaw_types.rs` use-after-move** — `browse_url: String` used twice in `JobStarted` mapping. Fixed by borrowing `&browse_url` in `json!()` macro, moving into `url: Some(browse_url)`.
2. **`ironclaw_bridge.rs` session path** — `LlmSessionManager` defaulted to `~/.ironclaw/session.json`. Fixed to use Scrappy's state dir: `state_dir.join("ironclaw_session.json")`.

---

## 0. Context: The Consumer App (Scrappy)

IronClaw is being integrated into **Scrappy**, an existing, mature Tauri v2 desktop
application. Scrappy is NOT a greenfield project — it has ~20,000 lines of Rust
backend code, a full React/TypeScript frontend (66+ components), and a
sophisticated module structure.

### 0.1 What Scrappy Already Has (Do NOT Duplicate)

| Scrappy System | Description | IronClaw Implication |
|---|---|---|
| **SecretStore** (`secret_store.rs`, 141 LOC) | macOS Keychain via `security-framework`. All API keys in single JSON blob (`com.schack.scrappy`/`api_keys`). | IronClaw accepts key strings via `Config`, does NOT access Scrappy's Keychain |
| **Multi-Engine Inference** (`engine/`, 72 KB) | `InferenceEngine` trait: LlamaCpp, MLX, vLLM, Ollama backends | IronClaw points at Scrappy's inference URL, does NOT spawn engines |
| **Sidecar Management** (`sidecar.rs`, 54 KB) | `SidecarManager` supervises llama-server, whisper-server, embedding-server | IronClaw should NOT manage inference sidecars |
| **Model Management** (`model_manager.rs`, 32 KB) | GGUF download, list, delete. HuggingFace Hub discovery | Not IronClaw's domain |
| **Native Rig Agent** (`rig_lib/`, ~155 KB) | Full agentic pipeline for Chat tab (separate from OpenClaw tab) | IronClaw powers OpenClaw tab only. They coexist |
| **RAG Pipeline** (`rag.rs` + `vector_store.rs`) | Document ingestion, usearch, ONNX reranking | Scrappy-side |
| **Image Generation** (`image_gen.rs`, 32 KB) | Local (sd.cpp, FLUX) + cloud (fal.ai) | Scrappy-side |
| **TTS/STT** (`tts.rs` + `stt.rs`) | Piper TTS, whisper transcription | Scrappy-side |
| **Config** (`config.rs`, 21 KB) | `UserConfig` with 40+ settings | Separate from IronClaw's `Config` |
| **Tauri IPC** (`lib.rs`, 499 LOC, 58+ commands) | All Tauri command registration, specta bindings, global shortcut, tray | Scrappy adds its own commands calling IronClaw |
| **Frontend** (66 React components, 600+ KB TS) | Full UI: Chat, OpenClaw, Imagine Studio, Settings, Projects, Spotlight | NOT the IronClaw embedded web UI |

### 0.2 What IronClaw Replaces

Today, Scrappy runs a **Node.js OpenClaw engine as a child process** communicating
over WebSocket. IronClaw replaces this entire subsystem:

```
CURRENT (being replaced):
  Scrappy (Rust/Tauri)
    └── SidecarManager spawns Node.js ──[WS:18789]──► openclaw-engine (npm)
         └── ws_client.rs ◄──[WsFrame events]── normalizer.rs ──► emit("openclaw-event")

TARGET (with IronClaw):
  Scrappy (Rust/Tauri)
    └── depends on ironclaw crate (in-process, no WS, no sidecar)
         └── Direct API calls → ironclaw::api::* → Result<T, E>
         └── Streaming events → TauriChannel → emit("openclaw-event")
```

**Files being deleted** (~90 KB of WS bridge code):

| File | Size | Purpose | Replaced By |
|---|---|---|---|
| `openclaw/ws_client.rs` | 26 KB | WS actor, handshake, RPC correlation | Direct Rust function calls |
| `openclaw/normalizer.rs` | 27 KB | Parse JSON events, map to `UiEvent` | IronClaw events are typed |
| `openclaw/frames.rs` | 4 KB | WS frame envelope types | Eliminated |
| `openclaw/ipc.rs` | 14 KB | Reverse-RPC for MCP tool calls | IronClaw calls tools directly |
| `openclaw/fleet.rs` | 13 KB | Fleet status polling | Direct `AppComponents` calls |
| `openclaw/commands/mod.rs` | 13 KB | `OpenClawManager` (process spawn) | `State<Arc<AppComponents>>` |
| Node.js sidecar | ~120 MB | Node.js binary + npm deps | Eliminated entirely |

### 0.3 Dual-Agent Architecture

| Concern | Chat Tab (Rig Agent) | OpenClaw Tab (IronClaw) |
|---|---|---|
| Entry point | `chat_stream` Tauri command | `openclaw_send_message` Tauri command |
| Agent loop | `Orchestrator::run_turn()` (Rhai ReAct) | IronClaw `Agent` background task |
| Streaming | `tauri::Channel<StreamChunk>` | `TauriChannel` → `app.emit("openclaw-event")` |
| Tools | Rhai sandbox + MCP + host tools | IronClaw `ToolRegistry` + `SafetyLayer` |
| Database | Scrappy's SQLite (sqlx) | IronClaw's libSQL (separate file) |

### 0.4 The Frontend Event Contract

The React frontend listens for `"openclaw-event"` with a typed `UiEvent` enum:

```typescript
type UiEvent =
  | { kind: "Connected"; protocol: number }
  | { kind: "Disconnected"; reason: string }
  | { kind: "SessionList"; sessions: UiSession[] }
  | { kind: "History"; session_key: string; messages: UiMessage[]; has_more: boolean }
  | { kind: "AssistantDelta"; session_key: string; run_id?: string; message_id: string; delta: string }
  | { kind: "AssistantSnapshot"; session_key: string; run_id?: string; message_id: string; text: string }
  | { kind: "AssistantInternal"; session_key: string; run_id?: string; message_id: string; text: string }
  | { kind: "AssistantFinal"; session_key: string; run_id?: string; message_id: string; text: string; usage?: UiUsage }
  | { kind: "ToolUpdate"; session_key: string; run_id?: string; tool_name: string;
      status: "started"|"stream"|"ok"|"error"; input: any; output: any }
  | { kind: "RunStatus"; session_key: string; run_id?: string;
      status: "started"|"in_flight"|"ok"|"error"|"aborted"; error?: string }
  | { kind: "ApprovalRequested"; approval_id: string; session_key: string; tool_name: string; input: any }
  | { kind: "ApprovalResolved"; approval_id: string; session_key: string; approved: boolean }
  | { kind: "Error"; code: string; message: string; details: any }
  | { kind: "CanvasUpdate"; session_key: string; content: string; content_type: "html"|"json" };

type UiSession = { session_key: string; title?: string; updated_at_ms?: number; source?: string };
type UiMessage = { id: string; role: "user"|"assistant"|"tool"|"system"; ts_ms: number; text: string };
type UiUsage = { input_tokens: number; output_tokens: number; total_tokens: number };
```

### 0.5 Secrets Bridge Architecture

Scrappy's `SecretStore` is the **single source of truth** for all API keys.
IronClaw does NOT maintain its own keychain. Keys flow as config strings:

```
Scrappy startup:
  1. keychain::load_all()  → reads JSON blob from macOS Keychain → Mutex<HashMap>
  2. For IronClaw init: extract relevant keys as String values
  3. Pass into ironclaw Config struct
  4. IronClaw uses keys directly — no cross-Keychain access
```

**Grant flags** are per-provider booleans stored in `identity.json`. A key
existing in the Keychain does NOT mean IronClaw can use it — only keys where
both `has_key = true` AND `granted = true` should be passed.

---

## 1. Architecture Decision: Hybrid API (Spawn + Direct)

### The Problem With Pure Approaches

IronClaw's `Agent::run()` is currently a **blocking event loop** — it reads
messages from a `MessageStream` and processes them internally. The `handle_message()`
method is `pub(super)`, not callable from outside the crate.

**Three integration approaches were considered:**

| | Channel-Feed | Pure Direct API | Hybrid (Chosen) |
|---|---|---|---|
| Pattern | Push into mpsc → Agent loop | Expose public API, await result | Validate → spawn → return Ok |
| Chat send | Fire-and-forget | **Blocks 10-30s** (entire turn) | Returns in 5ms, streams via events |
| Error handling | No Result, errors are events | Each call returns Result | Validation errors = Result, turn errors = events |
| Status queries | Queued behind messages | Direct lookup, sub-ms | Direct lookup, sub-ms |
| IronClaw changes | Minimal | Must expose API + refactor run() | Must expose API + refactor run() |

**Pure Direct API was rejected because `handle_message_external()` runs the
ENTIRE agent turn** — LLM calls (5-30s), tool execution (seconds each), multiple
agentic loop iterations. If the Tauri command awaits this, the UI freezes.
No streaming, no thinking indicator, no tool cards appearing.

IronClaw's own web gateway already uses the correct pattern: `chat_send_handler()`
pushes the message and returns `202 Accepted` immediately. Response streams via SSE.

### The Hybrid Pattern

Different operation types use different call patterns:

| Operation Type | Pattern | Examples | Why |
|---|---|---|---|
| **Agent turns** | Validate → `tokio::spawn` → return Ok | `send_message`, `resolve_approval` | Turn takes 5-30s, must not block |
| **Queries** | `await` → return Result | `list_sessions`, `get_config`, `read_memory` | Instant, sub-ms |
| **Simple writes** | `await` → return Result | `delete_session`, `write_file`, `set_config` | Fast, user needs confirmation |
| **Control** | Set flag → return Ok | `abort` | Must be instant |

### What the User Experiences

```
Click Send ──► 5ms ──► "Message accepted" (button re-enables, command returned Ok)
                              │
              50ms ──► 🔵 "Thinking..." indicator    (RunStatus event via TauriChannel)
             200ms ──► "Let me " tokens streaming    (AssistantDelta events)
             800ms ──► 🔧 Tool card appears           (ToolUpdate "started" event)
            1500ms ──► Tool result fills in           (ToolUpdate "ok" event)
            2000ms ──► More tokens streaming          (AssistantDelta events)
            3000ms ──► ✅ Response complete            (AssistantFinal event)
```

Every state transition is visible. The UI never freezes. Errors surface
immediately (validation errors as Result, turn errors as Error events).

### The Architecture Diagram

```
  ┌──────────────────────────────────────────────────────────────────────┐
  │ AGENT TURNS (spawn-and-return)                                      │
  │                                                                     │
  │  UI: Send ──► Tauri Command ──► validate ──► tokio::spawn(turn)     │
  │                    │                              │                 │
  │              returns Ok(msg_id)              runs in background      │
  │              in ~5ms                               │                │
  │                                                    ▼                │
  │  UI: Stream ◄── app.emit("openclaw-event") ◄── TauriChannel         │
  │  tokens in       (AssistantDelta,                  │                │
  │  real-time        ToolUpdate,                handles response,      │
  │                   ApprovalRequested,          tools, approvals       │
  │                   AssistantFinal)                                   │
  ├─────────────────────────────────────────────────────────────────────┤
  │ QUERIES & WRITES (direct await)                                     │
  │                                                                     │
  │  UI: Request ──► Tauri Command ──► ironclaw::api::sessions::list()  │
  │                       │                           │                 │
  │                 returns Result<T,E>          direct struct read      │
  │                 in ~1ms                      or DB query             │
  └──────────────────────────────────────────────────────────────────────┘
```

---

## 2. What IronClaw Must Build / Refactor

### 2.1 Expose Public API Surface (`ironclaw::api` module) — CRITICAL

This is the **single most important deliverable**. Extract business logic from
Axum-coupled handlers in `channels/web/handlers/` and from `pub(super)` Agent
methods into freestanding, framework-agnostic public functions.

**Current state:** Business logic is locked inside:
- Axum handlers (`State<Arc<GatewayState>>` → `Json<T>`)
- Agent methods marked `pub(super)` (unreachable from outside the `agent` module)

**Target state:** A new `ironclaw::api` module with pure Rust functions that
accept component references and return typed `Result<T, E>`:

```rust
// NEW: ironclaw/src/api/mod.rs
pub mod chat;
pub mod sessions;
pub mod memory;
pub mod config;
pub mod extensions;
pub mod skills;
pub mod routines;
pub mod system;
```

#### `api/chat.rs` — The Core Integration Point (Spawn-and-Return)

The critical design: `send_message()` validates input, spawns the agent turn
as a background task, and returns `Ok(message_id)` immediately. The turn
runs asynchronously — streaming events flow back through `TauriChannel`.

**This matches IronClaw's own web gateway pattern** (`chat_send_handler` returns
`202 Accepted` and the response streams via SSE).

```rust
// NEW: ironclaw/src/api/chat.rs

use std::sync::Arc;
use crate::agent::Agent;
use crate::agent::submission::Submission;
use crate::channels::{IncomingMessage, StatusUpdate};
use uuid::Uuid;

/// Send a user message and kick off an agent turn.
///
/// Returns immediately with the message ID. The actual agent turn
/// runs as a background tokio task. Streaming events (deltas, tool
/// updates, approvals, final response) are emitted through the
/// Channel trait during processing.
///
/// When `deliver` is false, the message is added to history but
/// does NOT trigger an agent turn (boot sequences, context injection).
///
/// # Error handling
/// - Validation errors (empty message, bad session) → returned as Err
/// - Turn errors (LLM down, tool failure) → emitted as Error events
pub async fn send_message(
    agent: &Arc<Agent>,
    session_key: &str,
    message: &str,
    deliver: bool,
) -> Result<SendMessageResult, ApiError> {
    // 1. Validate (instant)
    if message.trim().is_empty() {
        return Err(ApiError::InvalidInput("Empty message".into()));
    }

    let incoming = IncomingMessage::new("tauri", "local_user", message)
        .with_thread(session_key);
    let msg_id = incoming.id;

    if !deliver {
        // Silent inject: add to history, skip agent turn
        agent.inject_context(&incoming).await?;
        return Ok(SendMessageResult {
            message_id: msg_id,
            triggered_turn: false,
        });
    }

    // 2. Spawn the agent turn — DO NOT AWAIT
    let agent_clone = Arc::clone(agent);
    let session_key_owned = session_key.to_string();
    tokio::spawn(async move {
        if let Err(e) = agent_clone.handle_message_external(&incoming).await {
            tracing::error!("Agent turn failed: {}", e);
            agent_clone.channels.send_status_all(
                StatusUpdate::Error {
                    message: e.to_string(),
                    code: Some(e.error_code()),
                },
                &serde_json::json!({ "session_key": session_key_owned }),
            ).await.ok();
        }
    });

    // 3. Return immediately — UI gets instant acknowledgment (~5ms)
    Ok(SendMessageResult {
        message_id: msg_id,
        triggered_turn: true,
    })
}

/// Resolve a tool approval request.
pub async fn resolve_approval(
    agent: &Arc<Agent>,
    session_key: &str,
    approval_id: &str,
    approved: bool,
    always: bool,
) -> Result<(), ApiError> {
    let request_id = Uuid::parse_str(approval_id)?;
    let submission = Submission::ExecApproval {
        request_id,
        approved,
        always,
    };
    let content = serde_json::to_string(&submission)?;
    let msg = IncomingMessage::new("tauri", "local_user", content)
        .with_thread(session_key);

    let agent_clone = Arc::clone(agent);
    let session_key_owned = session_key.to_string();
    tokio::spawn(async move {
        if let Err(e) = agent_clone.handle_message_external(&msg).await {
            tracing::error!("Approval processing failed: {}", e);
            agent_clone.channels.send_status_all(
                StatusUpdate::Error {
                    message: e.to_string(),
                    code: Some("approval_failed".into()),
                },
                &serde_json::json!({ "session_key": session_key_owned }),
            ).await.ok();
        }
    });

    Ok(())
}

/// Abort a running chat turn.
pub async fn abort(
    agent: &Arc<Agent>,
    session_key: &str,
) -> Result<(), ApiError> {
    agent.cancel_turn(session_key).await?;
    Ok(())
}
```

#### `api/sessions.rs` — Thread/Session Management

```rust
// NEW: ironclaw/src/api/sessions.rs

/// List all sessions with metadata (title, last activity, message count).
pub async fn list_sessions(
    session_manager: &SessionManager,
    db: Option<&dyn Database>,
    user_id: &str,
) -> Result<Vec<SessionInfo>, ApiError> { ... }

/// Delete a session and all its data.
pub async fn delete_session(
    session_manager: &SessionManager,
    db: Option<&dyn Database>,
    session_key: &str,
) -> Result<(), ApiError> { ... }

/// Get conversation history for a session (in-memory first, fall back to DB).
pub async fn get_history(
    session_manager: &SessionManager,
    db: Option<&dyn Database>,
    session_key: &str,
    limit: usize,
    before: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<HistoryResponse, ApiError> { ... }

/// Create a new empty session (thread).
pub async fn create_session(
    session_manager: &SessionManager,
    user_id: &str,
) -> Result<SessionInfo, ApiError> { ... }
```

#### `api/memory.rs` — Workspace File Operations

```rust
// NEW: ironclaw/src/api/memory.rs

/// Read a workspace file (SOUL.md, MEMORY.md, BOOTSTRAP.md, etc.)
pub async fn get_file(workspace: &Workspace, path: &str) -> Result<String, ApiError> { ... }

/// Write a workspace file.
pub async fn write_file(workspace: &Workspace, path: &str, content: &str) -> Result<(), ApiError> { ... }

/// List all files in the workspace.
pub async fn list_files(workspace: &Workspace) -> Result<Vec<FileEntry>, ApiError> { ... }

/// Clear memory/identity/all.
pub async fn clear(workspace: &Workspace, target: ClearTarget) -> Result<(), ApiError> { ... }

/// Search workspace files by content.
pub async fn search(workspace: &Workspace, query: &str, limit: usize) -> Result<Vec<SearchResult>, ApiError> { ... }
```

#### `api/system.rs` — Status & Diagnostics

```rust
// NEW: ironclaw/src/api/system.rs

/// Engine status (maps to Scrappy's OpenClawStatus fields).
pub fn get_status(components: &AppComponents) -> EngineStatus { ... }

/// Health check.
pub fn health_check() -> HealthStatus { ... }

/// List available models from the LLM provider.
pub async fn list_models(llm: &dyn LlmProvider) -> Result<Vec<ModelInfo>, ApiError> { ... }

/// Tail recent logs.
pub fn tail_logs(broadcaster: &LogBroadcaster, limit: usize) -> Vec<LogEntry> { ... }
```

### 2.2 Make Agent Methods Public

```rust
impl Agent {
    // CHANGE: pub(super) → pub
    pub async fn handle_message_external(&self, message: &IncomingMessage) -> Result<Option<String>, Error> { ... }

    /// Inject a message into history without triggering a turn.
    pub async fn inject_context(&self, message: &IncomingMessage) -> Result<(), Error> { ... }

    /// Cancel a running turn directly — bypasses the message pipeline.
    pub async fn cancel_turn(&self, session_key: &str) -> Result<(), Error> { ... }
}
```

### 2.3 Separate Background Tasks from `Agent::run()`

```rust
impl Agent {
    /// Start background tasks (heartbeat, self-repair, cron, pruning).
    pub fn start_background_tasks(&self) -> BackgroundTasksHandle { ... }

    /// Gracefully stop all background tasks.
    pub async fn shutdown(&self, handle: BackgroundTasksHandle) { ... }

    // run() still exists for CLI mode — uses start_background_tasks() internally
    pub async fn run(self) -> Result<(), Error> { ... }
}
```

### 2.4 Accept External Config / Keys in `AppBuilder`

```rust
impl AppBuilder {
    /// Accept a pre-built secrets store, skipping init_secrets().
    pub fn with_secrets_store(mut self, store: Arc<dyn SecretsStore + Send + Sync>) -> Self { ... }
}
```

### 2.5 Feature-Gate Heavy Modules

```toml
[features]
default = ["postgres", "libsql", "html-to-markdown"]
desktop = ["libsql", "html-to-markdown"]
web-gateway = []
full = ["web-gateway", "repl", "tunnel", "docker-sandbox"]
```

### 2.6 Export All Necessary Types from `lib.rs`

```rust
pub mod api;
pub use config::Config;
pub use error::{Error, Result};
```

---

## 3. What Scrappy Builds (Consumer Side)

### 3.1 IronClaw Bridge (`ironclaw_bridge.rs`, ~100 lines)

Lifecycle management: init, config, Agent construction, shutdown.

### 3.2 Tauri Channel (`ironclaw_channel.rs`, ~120 lines)

Implements `ironclaw::channels::Channel` trait. Routes `StatusUpdate` events
to `app.emit("openclaw-event")` with proper session key tracking.

### 3.3 Rewritten Commands

All 66 commands rewritten from WS RPC → direct IronClaw API calls.

### 3.4 `lib.rs` Setup Changes

Async IronClaw spawn in setup + graceful shutdown in `RunEvent::Exit`.

---

## 4. The Full Command Contract

### Available Now (Maps to IronClaw Internals)

| Command | IronClaw API Function | Call Pattern | Source |
|---|---|---|---|
| `chat.send` | `api::chat::send_message()` | **🔥 Spawn** | → `tokio::spawn(Agent::handle_message_external())` |
| `chat.abort` | `api::chat::abort()` | **⚡ Control** | → Sets cancellation flag |
| `exec.approval.resolve` | `api::chat::resolve_approval()` | **🔥 Spawn** | → `tokio::spawn(Submission::ExecApproval)` |
| `chat.history` | `api::sessions::get_history()` | **📖 Query** | → `SessionManager` + DB |
| `sessions.list` | `api::sessions::list_sessions()` | **📖 Query** | → `SessionManager` + DB |
| `sessions.delete` | `api::sessions::delete_session()` | **✏️ Write** | → `SessionManager` + DB |
| `sessions.reset` | `api::sessions::clear_session()` | **✏️ Write** | → `SessionManager` |
| `memory.get` | `api::memory::get_file()` | **📖 Query** | → `Workspace::read_file()` |
| `memory.save` | `api::memory::write_file()` | **✏️ Write** | → `Workspace::write_file()` |
| `memory.clear` | `api::memory::clear()` | **✏️ Write** | → `Workspace` |
| `workspace.listFiles` | `api::memory::list_files()` | **📖 Query** | → `Workspace` |
| `workspace.getFile` | `api::memory::get_file()` | **📖 Query** | → `Workspace::read_file()` |
| `workspace.writeFile` | `api::memory::write_file()` | **✏️ Write** | → `Workspace::write_file()` |
| `config.get` | `api::config::get_config()` | **📖 Query** | → `AppComponents.config` |
| `config.set` | `api::config::set_config()` | **✏️ Write** | → DB settings store |
| `skills.list` | `api::skills::list_skills()` | **📖 Query** | → `SkillRegistry` |
| `skills.update` | `api::skills::toggle_skill()` | **✏️ Write** | → `SkillRegistry` |
| `skills.install` | `api::skills::install_skill()` | **✏️ Write** | → `SkillCatalog` |
| `cron.list` | `api::routines::list_routines()` | **📖 Query** | → `RoutineEngine` |
| `cron.run` | `api::routines::trigger_routine()` | **🔥 Spawn** | → `tokio::spawn(RoutineEngine::trigger())` |
| `status` | `api::system::get_status()` | **📖 Query** | → `AppComponents` (synchronous) |
| `logs.tail` | `api::system::tail_logs()` | **📖 Query** | → `LogBroadcaster` |
| `diagnostics` | `api::system::diagnostics()` | **📖 Query** | → Aggregated status |

**Legend:** 🔥 Spawn = returns immediately, work in background via events · 📖 Query = direct read, sub-ms · ✏️ Write = direct mutation, fast · ⚡ Control = sets flag, instant

### Not Applicable / Deferred

| Command | Status | Notes |
|---|---|---|
| `fleet.*` | ❌ Not in IronClaw | Fleet management not needed for desktop |
| `web.login.*` | ❌ Not in IronClaw | WhatsApp/Telegram flows |
| `canvas.*` | ❌ Not yet implemented | Future feature |
| `agents.list/spawn` | ❌ Not yet implemented | Sub-agent system |
| `update.run` | ❌ Not applicable | IronClaw updates via cargo-dist |
| `system.presence` | ❌ Not applicable | Multi-node concept |

---

## 5. Important Behavioral Details

### 5.1 The `deliver` Flag

`chat.send` accepts `deliver: bool`. When `deliver=false`:
- Message is added to history
- Does NOT trigger an agent turn
- Used for: boot sequences, date context injection, silent memory updates

### 5.2 Silent Reply Handling (`NO_REPLY` / `NO_REPL`)

IronClaw uses `NO_REPLY` token (in `llm/reasoning.rs`). Scrappy uses `NO_REPL`.
The agent loop already suppresses these at line 720 of `agent_loop.rs`. For Tauri,
IronClaw should **emit raw text** and let Scrappy apply its own sanitization.

### 5.3 Boot Sequence Protocol

On OpenClaw tab open, Scrappy sends a `SYSTEM_BOOT_SEQUENCE` message composed from
workspace files (SOUL.md, MEMORY.md, BOOTSTRAP.md). This is just a `chat.send()`
with `deliver=true`.

### 5.4 Token Sanitization

Scrappy's `normalizer.rs` has 10 compiled regexes for stripping ChatML/Jinja
tokens. This stays in Scrappy. IronClaw emits raw LLM output.

### 5.5 Security: Zeroize on Drop

Scrappy's `OpenClawConfig` implements `Drop` with `zeroize()` on API key fields.
IronClaw uses `secrecy::SecretString` which zeroizes on drop. Compatible approaches.

### 5.6 Model Selection & Fallback

IronClaw's `build_provider_chain()` implements: primary → retry → smart routing →
failover → circuit breaker → cache.

### 5.7 Specta TypeScript Bindings

All Tauri command types must be `specta::Type`. IronClaw types should be **wrapped
in Scrappy-side DTOs** that derive `specta::Type`.

### 5.8 Rust Edition

IronClaw uses Rust edition **2024** (MSRV 1.92). Scrappy uses edition **2021**.
This is not a blocker (editions are per-crate in a workspace).

### 5.9 MCP Reverse-RPC

With IronClaw in-process, the MCP reverse-RPC is eliminated. IronClaw calls tools
directly via `ToolRegistry`.

---

## 6. Database Strategy

| Database | Engine | Content | Managed By |
|---|---|---|---|
| `openclaw.db` | SQLite (sqlx) | Chat tab conversations, documents, vectors, images | Scrappy |
| `ironclaw.db` | libSQL | OpenClaw tab sessions, memory, workspace, cron, skills | IronClaw |

Separate databases serving different domains. Do NOT unify initially.

**SQLite coexistence:** Resolved via patched `libsql-0.6.0` in `patches/libsql-0.6.0/`.

---

## 7. New Code Summary

### IronClaw Side

| Change | Est. Lines | Complexity |
|---|---|---|
| `ironclaw::api` module (8 submodules) | ~500 | Medium |
| Make `handle_message()` public + new methods | ~50 | Low |
| Extract background tasks + shutdown | ~60 | Medium |
| `StatusUpdate::Error` variant | ~5 | Low |
| Feature flags + `#[cfg]` | ~50 | Low |
| **Total IronClaw changes** | **~765** | |

### Scrappy Side

| Change | Est. Lines | Complexity |
|---|---|---|
| `ironclaw_bridge.rs` | ~100 | Low |
| `ironclaw_channel.rs` | ~80 | Low |
| Rewrite 50+ commands | ~500 | Medium |
| Delete WS bridge files | -90 KB | Low |
| **Total Scrappy changes** | **~720 new, ~90 KB deleted** | |

---

## 8. Implementation Checklist

All 15 phases complete. See Implementation Progress table above.

**Total estimated time: 7-10 days · Elapsed: DONE (all 15 phases complete)**

---

## 9. Post-Integration Work Completed

### Work Stream C — Wire Cloud Backends ✅
All 3 live Tauri commands (`tts_synthesize`, `transcribe_audio`, `imagine_generate`) now route through `InferenceRouter`. Frontend badges and voice selectors implemented.

### Work Stream D — Cloud Model Discovery ✅
12-provider live model discovery (OpenAI, Anthropic, Gemini, Groq, OpenRouter, Mistral, xAI, Together, Cohere, ElevenLabs, Stability, static). Frontend integration in ModelBrowser, ModelSelector, InferenceModeTab. Context size propagation to all chat backends.

### Work Stream A — Cloud Storage (A1–A6 ✅ All Complete)
- **A1 Foundation ✅:** `CloudProvider` trait, S3 impl (opendal), AES-256-GCM encryption, DB snapshots, `ArchiveManifest`, `CloudManager` state + 8 Tauri commands. 13 tests.
- **A2 Migration Engine ✅:** `run_to_cloud()` (7-phase), `run_to_local()` (6-phase), cancellable+resumable, spot-check verification. 683 LOC.
- **A3 FileStore Abstraction ✅:** `FileStore` struct (310 LOC, 12 methods), 13 fs call sites migrated across 5 modules. sessions.rs skipped (IronClaw workspace, outside scope).
- **A4 Additional Providers ✅:** All 7 providers (S3 end-to-end, iCloud/GDrive/Dropbox with frontend cards, OneDrive/WebDAV/SFTP backend).
- **A5 Frontend UI ✅:** StorageTab, provider picker, migration progress dialog. `ApprovalCard` extended to 3-tier. Storage breakdown includes Agent Database.
- **A6 Tests & Polish ✅:** Integration tests, IronClaw DB snapshot in migration Phase 2b (VACUUM INTO + encrypt + upload).

---

## 10. Critical Notes for the IronClaw Agent

1. **You are a library, not the app.** IronClaw replaces only the OpenClaw
   Node.js engine — everything else in Scrappy stays.

2. **All 15 integration phases are DONE.** The `ironclaw::api` module, feature
   flags, public Agent methods, background task separation — all implemented
   and verified. No spec work remains.

3. **Cloud Storage A1–A6 is DONE.** All phases complete on both sides. IronClaw
   exposes `api::system::snapshot_database()` for DB migration. No further IronClaw
   work needed for cloud storage.

4. **IronClaw's libSQL DB is included in cloud migrations.** Scrappy calls
   `api::system::snapshot_database()` during migration Phase 2b. Uses `VACUUM INTO`
   for a consistent snapshot, then encrypts and uploads as `db/ironclaw.db.enc`.

5. **Workspace files are NOT in cloud migrations.** `SOUL.md`, `MEMORY.md`, and
   sessions live in IronClaw's workspace directory and are managed by IronClaw's
   `Workspace` API. They are intentionally excluded from Scrappy's FileStore.

6. **Agent turns are spawned, NOT awaited.** This remains the core UX invariant.
   `send_message()` and `resolve_approval()` return in ~5ms.

7. **Two databases is still correct.** Chat SQLite (Scrappy) + agent libSQL
   (IronClaw). No unification planned or needed.

8. **The UiEvent contract is sacred.** Frontend React components listen for
   specific shapes. IronClaw events must continue mapping to these.

---

## 11. Critical Notes for the Scrappy Agent

1. **All major initiatives are COMPLETE.** Cloud Storage (A1–A6), InferenceRouter,
   Model Discovery, FileStore, Hardware Bridge, and Voice/Talk Mode integration are
   all done. Only deferred items remain (Sherpa-ONNX, Skill Deps).

2. **Zero-downtime secret refresh is available.** Call
   `ironclaw::api::config::refresh_secrets(secrets, user_id)` instead of stop→start
   when the user updates an API key. This re-reads all secrets and updates the
   config overlay atomically.

3. **`sessions.rs` stays on raw filesystem.** These 18 call sites operate on
   IronClaw's workspace directory, which is outside `app_data_dir`. They are
   NOT migrated to FileStore.

4. **Error handling is split.** Validation errors → `Err()` from Tauri commands.
   Turn errors → `UiEvent::Error` events. Both must surface in UI.

5. **Token sanitization stays in Scrappy.** IronClaw emits raw LLM output.
   Apply regex sanitizers before rendering.

6. **Secrets flow one way.** Keychain → grant flags → `ironclaw::Config` →
   `AppBuilder`. IronClaw never reads Scrappy's Keychain directly.

7. **Specta regen needed.** Run `cargo tauri dev` to regenerate `bindings.ts`
   with new types from cloud storage, model discovery, and inference mode.

8. **Pre-existing lint note.** `rag.rs:1096` has a `non-primitive cast` error
   from rust-analyzer. This is pre-existing and unrelated to recent changes.
   The actual build compiles clean with 0 errors, 0 warnings.

---

## 12. Post-Integration Status & Cross-Project Coordination

> Last updated: 2026-03-02 07:10 CET — **Integration complete on both sides. Extended thinking shipped end-to-end. Scrappy can uncomment `with_tool_bridge()` to wire sensor tools.**

### 12.1 IronClaw Internal Tasks — All Complete

| Task | What Was Done |
|------|---------------|
| **Self-Repair Wiring** | `with_store()` wired into `agent_loop.rs` — failure tracking persists to DB |
| **QR Code Rendering** | Added `qrcode = "0.14"`, real Unicode half-block QR matrix in terminal |
| **Dead Code Audit** | All 41 `#[allow(dead_code)]` annotations reviewed, 4 removed, 2 comments improved |
| **Paragraph Chunking** | `ChunkingStrategy` enum (`Fixed`/`Paragraph`), `chunk()` dispatch function |
| **Evaluation Framework** | `RuleBasedEvaluator` runs on every job completion, results logged + persisted |
| **DB Snapshotting (X2)** | `Database::snapshot()` + `db_path()` on trait. WAL checkpoint + file copy for libsql. `api::system::snapshot_database()` exposed for cloud migration. |
| **Voice EnergyDetector (T8)** | `cpal = "0.15"` behind `voice` feature flag. RMS energy detection on dedicated OS thread. Headless-only. |
| **WASM Introspection (T7)** | Component type introspection: detects export kinds, identifies WIT-compliant tools |

### 12.2 Cross-Project Tasks — Resolved

#### T5: Hardware Bridge — Internal Rust Trait (was WS RPC)

**Architecture change:** Since IronClaw is now in-process (not a remote WS server), the Hardware Bridge uses an internal Rust trait instead of WebSocket RPC. This is simpler and faster.

**Scrappy confirmed:** No WS handler exists. The old WS bridge was deleted in Phase 4. An internal trait/callback is the correct approach.

**Implemented:** `hardware_bridge.rs` (380 LOC, 7 tests):
- `ToolBridge` trait — Scrappy implements this to provide sensor access
- `BridgedTool` — wraps bridge calls with 30s timeout + session approval caching
- `SessionApprovals` — in-memory per-session approval tracker (3 tiers: Deny / Allow Once / Allow Session)
- `SensorRequest` / `SensorResponse` — typed request/response structs
- `create_bridged_tools()` — creates camera, mic, screen tools for registration

**Scrappy integration needed:** Implement `ToolBridge` trait using the existing `ApprovalCard` component (extend for 3-tier approval). Pass the bridge via `AppBuilder` at startup.

#### T8: Voice & Talk Mode — Dual Architecture

**Scrappy confirmed:**
- Desktop: Scrappy owns mic capture via browser MediaRecorder API. No cpal conflict.
- STT: Local whisper at `localhost:53757/v1/audio/transcriptions` (OpenAI-compatible). Auth via Bearer token.
- Cloud: `InferenceRouter` can select OpenAI Whisper API or Deepgram.

**Implemented:**
- `TranscriptionBackend::WhisperHttp` — new variant for local sidecar STT
- `transcribe_whisper_http()` — calls local endpoint with multipart/form-data
- `TalkModeTool::execute()` — auto-selects backend: `WHISPER_HTTP_ENDPOINT` env → local sidecar, else → OpenAI cloud
- Voice wake: cpal-based RMS energy detection behind `voice` feature flag (headless only)

#### X1: Cloud Storage — Complete on Both Sides

**Scrappy confirmed:** All 7 providers implemented (S3 end-to-end, iCloud/GDrive/Dropbox with frontend cards, OneDrive/WebDAV/SFTP backend only). CloudProvider trait abstracts all differences. IronClaw doesn't need provider helpers.

**Decision: Include ironclaw.db in cloud migrations.** Scrappy's recommendation: yes, users expect session history to survive device switches. IronClaw now exposes `api::system::snapshot_database()` which Scrappy's migration engine can call.

### 12.3 Scrappy Answers — Archive

> Questions from the previous version of this section, preserved for reference.

| # | Question | Answer | Action Taken |
|---|----------|--------|--------------|
| Q1 | WS handler for `tool.rpc.request`? | ❌ No WS handler | **Pivoted** to internal Rust trait (`ToolBridge`) |
| Q2 | Approval dialog? | ✅ `ApprovalCard.tsx` (binary only) | Bridge supports 3 tiers; Scrappy needs to extend card |
| Q3 | Sensor crates? | ❌ None linked | No conflict; sensors are host-side only |
| Q4 | `tool.rpc.response`? | ❌ No | Moot — internal trait returns directly |
| Q5 | Approval model? | Per-request, `always` param unused | `SessionApprovals` implemented in bridge |
| Q6 | cpal linked? | ❌ No | No conflict; audio via MediaRecorder |
| Q7 | Whisper endpoint? | ✅ `localhost:53757` | `WhisperHttp` backend added to talk_mode |
| Q8 | Mic ownership? | Scrappy (frontend) | cpal only in headless mode behind `voice` flag |
| Q9 | Cloud providers? | All 7 implemented | No IronClaw action needed |

### 12.4 Build Health

**Build:** 0 warnings, 0 errors (both default and `--features voice`)
**Tests:** 1,740 passing, 0 failures, 1 ignored

### 12.5 Integration Status — Both Sides Complete

**Scrappy completed (2026-03-01 16:00):**
- ✅ `TauriToolBridge` — implements `ToolBridge` trait with 3-tier approval, session permission cache, 5-min timeout
- ✅ `ApprovalCard.tsx` — 3 buttons: Allow Once (green), Allow Session (blue), Deny (red)
- ✅ `WHISPER_HTTP_ENDPOINT` — set to `http://127.0.0.1:53757/v1/audio/transcriptions` in `ironclaw_bridge.rs`
- ✅ Cloud DB snapshot — `VACUUM INTO` during migration Phase 2b, encrypted + uploaded as `db/ironclaw.db.enc`

**IronClaw exposed (2026-03-01 16:10):**
- ✅ `AppBuilder::with_tool_bridge(bridge)` — Scrappy can now inject the bridge at startup
- ✅ `AppComponents` includes `tool_bridge` and `session_approvals` fields
- ✅ `build_all()` auto-registers bridged sensor tools when a bridge is present

**Final wiring — CONFIRMED LIVE (2026-03-02):**

Scrappy confirmed both lines were already uncommented and operational:
```rust
// In ironclaw_bridge.rs — ALREADY LIVE:
pub use ironclaw::hardware_bridge::ToolBridge;       // ✅ Live
builder = builder.with_tool_bridge(tool_bridge.clone()); // ✅ Live
```

The 3-tier approval flow (ToolBridge → TauriToolBridge → ApprovalCard.tsx) is fully wired
and operational. IronClaw's prior report claiming "2 lines to uncomment" was stale.

| Item | Status |
|------|--------|
| Sherpa-ONNX keyword spotting | Deferred — scaffold exists, no ML model |
| X3: Skill Deps | Deferred — no skills use cross-crate deps |
| WS RPC for remote mode | Deferred — internal trait handles desktop |

### 12.6 Extended Thinking — Shipped (2026-03-02)

Full end-to-end extended thinking / chain-of-thought reasoning:

| Layer | What |
|-------|------|
| **Settings** | `thinking_enabled` + `thinking_budget_tokens` in `AgentSettings` with env var overrides (`AGENT_THINKING_ENABLED`, `AGENT_THINKING_BUDGET_TOKENS`) |
| **Config** | `AgentConfig` resolves thinking settings from `Settings` + env vars |
| **Core** | `ThinkingConfig` enum (Disabled/Enabled), `Copy` derive, builder methods on `CompletionRequest` / `ToolCompletionRequest` |
| **Reasoning** | `thinking` field on `ReasoningContext`, `thinking_content` on `RespondOutput`, both LLM paths wired |
| **Dispatcher** | Reads config → builds `ThinkingConfig` → emits thinking as `StatusUpdate::Thinking` |
| **SSE/WS** | `SseEvent::ReasoningContent` variant, mapped in both SSE and WS |
| **OpenAI-compat** | `reasoning_content` field on `OpenAiMessage` (skip_serializing_if = None) |
| **Rig Adapter** | `thinking_config_to_params()` maps to Anthropic/OpenAI-specific JSON params |
| **Tests** | 10 tests: 4 unit (thinking_config_to_params), 4 unit (extract_response reasoning), 2 integration (openai-compat API) |

**Scrappy thinking UI mapping (2026-03-02):** `StatusUpdate::Thinking(text)` now maps to
`AssistantInternal` in `ironclaw_types.rs` (was discarding the text). Rendered with 🧠 indicator.
Collapsible reasoning UI is P3.

### 12.7 Real Token-Level Streaming — Shipped (2026-03-02)

Replaces the previous simulated word-chunking with native token-level streaming:

| Layer | What |
|-------|------|
| **StreamChunk** | New enum: `Text`, `ReasoningDelta`, `ToolCall`, `ToolCallDelta`, `Done` |
| **StreamChunkStream** | `Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send>>` |
| **LlmProvider trait** | `complete_stream()` + `complete_stream_with_tools()` with default simulated fallback |
| **LlmProvider trait** | `supports_streaming()` flag for header reporting |
| **RigAdapter** | Native streaming via rig's `CompletionModel::stream()` + `async_stream` |
| **OpenAI-compat SSE** | `handle_streaming()` rewritten to consume `StreamChunkStream` |
| **Header** | `x-ironclaw-streaming: native` (RigAdapter) or `simulated` (fallback) |
| **Tests** | 3 unit tests: text-only, reasoning+tools, empty content |

**Impact for Scrappy:** The `/v1/chat/completions` endpoint now delivers per-token SSE chunks
instead of word-boundary simulated chunks. The SSE format is unchanged (standard OpenAI
streaming protocol), so no Scrappy-side changes are needed.

### 12.8 Scrappy Hot-Reload Fix (2026-03-02)

Scrappy uncommented `refresh_secrets()` hot-reload (Enhancement 2B Tier 1) and fixed
the API signature to `(secrets_store, "local_user")`. The enhancement table in
`ironclaw_library_roadmap.md` has been updated by Scrappy: 2A/2B/2C all marked done,
Extended Thinking row added.

### 12.9 Media Pipeline — Shipped (2026-03-02)

New `src/media/` module (4 files, ~1,200 LOC) for processing binary attachments:

| Component | File | What |
|-----------|------|------|
| **Types** | `media/types.rs` | `MediaType` enum, `MediaContent` struct, `MediaExtractor` trait, `MediaPipeline` |
| **Image** | `media/image.rs` | Dimension detection (PNG/GIF/WebP/JPEG), base64 data-URI, LLM multimodal formatting |
| **PDF** | `media/pdf.rs` | BT/ET text block parsing, parenthesized string extraction, readable-sequence fallback |
| **Audio** | `media/audio.rs` | Whisper HTTP endpoint integration, multipart upload, duration estimation |
| **Channel** | `channel.rs` | `IncomingMessage.attachments: Vec<MediaContent>` field added |
| **Tests** | All | 31 unit tests across all modules |

**Impact for Scrappy:** Channels can now pass media attachments on `IncomingMessage`.
Scrappy's `ironclaw_channel.rs` can populate `attachments` from Tauri file drops or
clipboard images. The `MediaPipeline.extract()` method handles routing to the correct
extractor based on MIME type.

### 12.10 Infrastructure — Shipped (2026-03-02)

| Component | File | What |
|-----------|------|------|
| **D2: ChannelHealthMonitor** | `channels/health_monitor.rs` | Periodic `health_check_all()`, failure counting, auto-restart with configurable max attempts (3) and cooldown (30s) |
| **D3: ConfigWatcher** | `config/watcher.rs` | Polling-based file watcher (2s default), mtime comparison, debounce (500ms), `broadcast::channel` for subscribers |

**Impact for Scrappy:** `ChannelHealthMonitor` can be started as a background task to
auto-restart failing channels. `ConfigWatcher` can watch `~/.ironclaw/config.toml` and
trigger config reload on changes.

### 12.11 Integration Wiring — Shipped (2026-03-02)

All three new subsystems are now wired into the agent's lifecycle:

| Subsystem | Where | How |
|-----------|-------|-----|
| **MediaPipeline** | `thread_ops.rs:process_user_input()` | Extracts text from `message.attachments` and prepends to user content before LLM call |
| **ChannelHealthMonitor** | `agent_loop.rs:start_background_tasks()` | Started/stopped alongside self-repair and session pruning |
| **ConfigWatcher** | `agent_loop.rs:run()` | Watches `~/.ironclaw/config.toml`, logs changes, stopped on shutdown |

**Impact for Scrappy:** Scrappy can now attach files via `IncomingMessage.attachments`
and they will be automatically processed before the LLM call. Channel health is
monitored continuously. Config file changes are detected and logged.
