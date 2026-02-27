# ThinClaw Tauri App — Integration Specification

> **Date:** 2026-02-27 · **Base:** IronClaw v0.12.0 · **Target:** Tauri v2 desktop app (Scrappy)
> **Architecture:** Hybrid API — spawn-and-return for agent turns, direct for queries
> **Approach:** IronClaw as library crate, refactored to expose public API surface

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
    //    Response, tool cards, streaming all arrive as events
    //    through TauriChannel::respond() and send_status()
    let agent_clone = Arc::clone(agent);
    let session_key_owned = session_key.to_string();
    tokio::spawn(async move {
        if let Err(e) = agent_clone.handle_message_external(&incoming).await {
            tracing::error!("Agent turn failed: {}", e);
            // Emit error through channel infrastructure so the UI
            // receives it as UiEvent::Error (not just a log line).
            // Without this, a failed turn leaves an infinite spinner.
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
///
/// Spawns the approval processing as a background task (the resumed
/// agent turn may involve further LLM calls and tool executions).
/// Returns immediately.
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

    // Spawn — approval may resume a multi-step tool chain
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
///
/// Uses `Agent::cancel_turn()` for direct cancellation — skips the
/// full message pipeline (hooks, submission parsing, session resolution).
/// Just locks the session and sets the thread's interrupt flag.
/// Returns immediately; the running turn stops at its next yield point.
pub async fn abort(
    agent: &Arc<Agent>,
    session_key: &str,
) -> Result<(), ApiError> {
    // Direct cancellation — no hook overhead, no message parsing.
    // Internally: session.lock() → thread.interrupt() → done.
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
pub async fn get_file(
    workspace: &Workspace,
    path: &str,
) -> Result<String, ApiError> { ... }

/// Write a workspace file.
pub async fn write_file(
    workspace: &Workspace,
    path: &str,
    content: &str,
) -> Result<(), ApiError> { ... }

/// List all files in the workspace.
pub async fn list_files(
    workspace: &Workspace,
) -> Result<Vec<FileEntry>, ApiError> { ... }

/// Clear memory/identity/all.
pub async fn clear(
    workspace: &Workspace,
    target: ClearTarget, // "memory" | "identity" | "all"
) -> Result<(), ApiError> { ... }

/// Search workspace files by content.
pub async fn search(
    workspace: &Workspace,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, ApiError> { ... }
```

#### `api/system.rs` — Status & Diagnostics

```rust
// NEW: ironclaw/src/api/system.rs

/// Engine status (maps to Scrappy's OpenClawStatus fields).
pub fn get_status(components: &AppComponents) -> EngineStatus {
    EngineStatus {
        engine_running: true,
        setup_completed: components.workspace.is_some(),
        tool_count: components.tools.count(),
        active_extensions: components.extension_manager
            .as_ref().map(|m| m.active_count()).unwrap_or(0),
        model_name: components.llm.active_model_name(),
        db_connected: components.db.is_some(),
        // ... etc
    }
}

/// Health check.
pub fn health_check() -> HealthStatus { ... }

/// List available models from the LLM provider.
pub async fn list_models(
    llm: &dyn LlmProvider,
) -> Result<Vec<ModelInfo>, ApiError> { ... }

/// Tail recent logs.
pub fn tail_logs(
    broadcaster: &LogBroadcaster,
    limit: usize,
) -> Vec<LogEntry> { ... }
```

### 2.2 Make Agent Methods Public

The `Agent` struct needs these changes:

```rust
// In agent/agent_loop.rs — changes to existing code:

impl Agent {
    // CHANGE: pub(super) → pub
    // This is the key method that Scrappy calls through ironclaw::api
    pub async fn handle_message_external(
        &self,
        message: &IncomingMessage,
    ) -> Result<Option<String>, Error> {
        // Delegates to existing handle_message() which does:
        // 1. Parse submission
        // 2. Run inbound hooks
        // 3. Hydrate thread from DB
        // 4. Resolve session/thread
        // 5. Process based on submission type
        // 6. Run outbound hooks
        // 7. Return response
        self.handle_message(message).await
    }

    /// Inject a message into history without triggering a turn.
    /// Used for boot sequences, context updates, silent system messages.
    pub async fn inject_context(
        &self,
        message: &IncomingMessage,
    ) -> Result<(), Error> {
        let (session, thread_id) = self.session_manager
            .resolve_thread(&message.user_id, &message.channel, message.thread_id.as_deref())
            .await;
        self.persist_user_message(thread_id, &message.user_id, &message.content).await;
        Ok(())
    }

    /// Cancel a running turn directly — bypasses the message pipeline.
    ///
    /// This is faster than routing `/interrupt` through handle_message_external()
    /// because it skips hook chains, submission parsing, and session resolution.
    /// It directly locks the session and sets the thread's cancellation flag.
    ///
    /// Internally calls process_interrupt() which does:
    ///   session.lock() → thread.interrupt() → done
    pub async fn cancel_turn(
        &self,
        session_key: &str,
    ) -> Result<(), Error> {
        let (session, thread_id) = self.session_manager
            .resolve_thread("local_user", "tauri", Some(session_key))
            .await;
        self.process_interrupt(session, thread_id).await?;
        Ok(())
    }
}
```

### 2.3 Separate Background Tasks from `Agent::run()`

Currently `Agent::run()` bundles message processing AND background task management
(heartbeat, self-repair, cron, session pruning) into one blocking function.

**Refactor into two methods:**

```rust
impl Agent {
    /// Start background tasks (heartbeat, self-repair, cron, pruning).
    /// Returns a handle that can be used to shut them down.
    pub fn start_background_tasks(&self) -> BackgroundTasksHandle {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Spawn self-repair (lines 228-307 of current run())
        let repair_handle = { /* ... existing code ... */ };

        // Spawn session pruning (lines 310-319)
        let pruning_handle = { /* ... existing code ... */ };

        // Spawn heartbeat if enabled (lines 322-389)
        let heartbeat_handle = { /* ... existing code ... */ };

        // Spawn routine engine if enabled (lines 392-466)
        let routine_handle = { /* ... existing code ... */ };

        BackgroundTasksHandle {
            shutdown_tx,
            handles: vec![repair_handle, pruning_handle, /* ... */],
        }
    }

    /// Gracefully stop all background tasks.
    /// Called when the Tauri window closes or app quits.
    pub async fn shutdown(&self, handle: BackgroundTasksHandle) {
        handle.shutdown_tx.send(true).ok();
        for h in handle.handles {
            h.abort();
        }
        self.scheduler.stop_all().await;
    }

    // run() still exists for CLI mode — uses start_background_tasks() internally
    pub async fn run(self) -> Result<(), Error> {
        let bg = self.start_background_tasks();
        let mut message_stream = self.channels.start_all().await?;
        loop {
            // ... existing message loop ...
        }
        self.shutdown(bg).await;
        Ok(())
    }
}
```

### 2.4 Accept External Config / Keys in `AppBuilder`

`AppBuilder::init_secrets()` currently creates its own keychain store. For Tauri
integration, Scrappy passes keys as config strings:

```rust
// Scrappy-side: build config with keys from its own SecretStore
let mut config = ironclaw::Config::default();
config.llm.nearai.api_key = secret_store.get("anthropic");
config.llm.nearai.model = "anthropic/claude-sonnet-4-5".to_string();
config.llm.nearai.fallback_model = Some("openai/gpt-4o".to_string());

// Point at Scrappy's local inference server
if local_inference_enabled {
    config.llm.nearai.base_url = format!("http://127.0.0.1:{}/v1", sidecar_port);
    config.llm.nearai.model = "local/model".to_string();
}
```

**Optional: Add `with_secrets_store()` for richer integration:**

```rust
impl AppBuilder {
    /// Accept a pre-built secrets store, skipping init_secrets().
    pub fn with_secrets_store(
        mut self,
        store: Arc<dyn SecretsStore + Send + Sync>,
    ) -> Self {
        self.external_secrets = Some(store);
        self
    }
}

// In init_secrets():
if let Some(external) = self.external_secrets.take() {
    self.secrets_store = external;
    return Ok(());
}
// ... fall back to built-in keychain store
```

### 2.5 Feature-Gate Heavy Modules

IronClaw currently has only `postgres`, `libsql`, and `html-to-markdown` feature
flags. Add feature gates for heavy modules not needed in Tauri mode:

```toml
# Cargo.toml additions
[features]
default = ["postgres", "libsql", "html-to-markdown"]
# NEW: Minimal feature set for embedding in a Tauri app
desktop = ["libsql", "html-to-markdown"]
# NEW: Feature gates for heavy modules
web-gateway = []    # channels/web/server.rs, sse.rs, ws.rs, openai_compat.rs, static/
repl = []           # channels/repl.rs
tunnel = []         # tunnel/*
docker-sandbox = [] # sandbox/container.rs, orchestrator/*, worker/*
full = ["web-gateway", "repl", "tunnel", "docker-sandbox"]
```

Then in `lib.rs`:

```rust
// Keep unconditionally (needed by all modes):
pub mod agent;
pub mod api;           // NEW
pub mod app;
pub mod channels;      // Channel trait + types (NOT web server)
pub mod config;
pub mod context;
pub mod db;
pub mod error;
pub mod hooks;
pub mod llm;
pub mod safety;
pub mod secrets;
pub mod skills;
pub mod tools;
pub mod workspace;

// Feature-gated:
#[cfg(feature = "web-gateway")]
pub mod web_server;     // The Axum server and its handlers

#[cfg(feature = "repl")]
pub mod repl_channel;

#[cfg(feature = "tunnel")]
pub mod tunnel;

#[cfg(feature = "docker-sandbox")]
pub mod orchestrator;
#[cfg(feature = "docker-sandbox")]
pub mod worker;

// Always strip in desktop mode:
#[cfg(not(feature = "desktop"))]
pub mod boot_screen;
#[cfg(not(feature = "desktop"))]
pub mod setup;
```

**What MUST remain in `desktop` mode:**
- `channels/channel.rs` — the `Channel` trait
- `channels/web/types.rs` — 152 typed event DTOs (needed for serialization)
- `app.rs` — `AppBuilder` + `AppComponents`
- `agent/` — full agent runtime
- `tools/` — tool registry, safety layer
- `llm/` — provider chain
- `db/` — database trait + libSQL
- `secrets/` — `SecretsStore` trait (not concrete Keychain impl)
- `config/` — config types

### 2.6 Export All Necessary Types from `lib.rs`

Ensure public exports cover everything:

```rust
// ironclaw/src/lib.rs — required additions
pub mod api;  // NEW

pub use config::Config;
pub use error::{Error, Result};

pub mod prelude {
    pub use crate::api;
    pub use crate::app::{AppBuilder, AppBuilderFlags, AppComponents};
    pub use crate::agent::Agent;
    pub use crate::channels::{Channel, IncomingMessage, OutgoingResponse, StatusUpdate};
    pub use crate::config::Config;
    pub use crate::llm::LlmProvider;
    pub use crate::tools::ToolRegistry;
    pub use crate::workspace::Workspace;
}
```

---

## 3. What Scrappy Builds (Consumer Side)

### 3.1 IronClaw Bridge (`ironclaw_bridge.rs`, ~100 lines)

```rust
use std::sync::Arc;
use ironclaw::app::{AppBuilder, AppBuilderFlags, AppComponents};
use ironclaw::agent::{Agent, AgentDeps};
use ironclaw::channels::ChannelManager;
use ironclaw::config::Config;
use ironclaw::llm::SessionManager;

pub struct IronClawState {
    pub components: Arc<AppComponents>,
    pub agent: Arc<Agent>,
    pub channel: Arc<TauriChannel>,
    /// Wrapped in Mutex<Option<>> so shutdown can .take() ownership
    /// from Tauri's managed state (which is behind Arc).
    pub background: tokio::sync::Mutex<Option<ironclaw::agent::BackgroundTasksHandle>>,
}

pub async fn init_ironclaw(
    app: &tauri::AppHandle,
    secret_store: &crate::secret_store::SecretStore,
    user_config: &crate::config::UserConfig,
) -> Result<IronClawState, anyhow::Error> {
    // 1. Build IronClaw config from Scrappy's settings
    let app_data_dir = app.path().app_data_dir()?;
    let ironclaw_dir = app_data_dir.join("ironclaw");
    std::fs::create_dir_all(&ironclaw_dir)?;

    let mut config = Config::default();
    config.database.backend = ironclaw::config::DatabaseBackend::LibSql;
    config.database.libsql_path = Some(ironclaw_dir.join("ironclaw.db"));

    // 2. Pass API keys (only granted providers)
    inject_granted_keys(&mut config, secret_store, user_config);

    // 3. Point at Scrappy's local inference if enabled
    if user_config.local_inference_enabled {
        if let Some(port) = get_inference_port(app) {
            config.llm.nearai.base_url = format!("http://127.0.0.1:{}/v1", port);
            config.llm.nearai.model = "local/model".to_string();
        }
    }

    // 4. Build engine (same code path as CLI)
    let session = Arc::new(SessionManager::new());
    // NOTE: LogBroadcaster may live in channels::web which is behind the
    // web-gateway feature flag. If so, either:
    //   (a) Move LogBroadcaster to a shared module (recommended), or
    //   (b) Use a no-op stub: ironclaw::log::NullBroadcaster::new()
    // The actual import path depends on IronClaw's module layout.
    let log_broadcaster = Arc::new(ironclaw::log::LogBroadcaster::new());
    let builder = AppBuilder::new(
        config,
        AppBuilderFlags::default(),
        None,
        session,
        log_broadcaster,
    );
    let components = Arc::new(builder.build_all().await?);

    // 5. Create TauriChannel and Agent
    let tauri_channel = Arc::new(TauriChannel::new(app.clone()));
    let channel_manager = Arc::new(ChannelManager::new());
    channel_manager.register(tauri_channel.clone());  // clone Arc before move

    let agent = Arc::new(Agent::new(
        components.config.agent.clone(),
        AgentDeps { /* ... from components ... */ },
        channel_manager,
        /* heartbeat, hygiene, routine configs ... */
    ));

    // 6. Start background tasks (heartbeat, cron, self-repair)
    let background = agent.start_background_tasks();

    Ok(IronClawState {
        components, agent, channel: tauri_channel,
        background: tokio::sync::Mutex::new(Some(background)),
    })
}

fn inject_granted_keys(config: &mut Config, store: &SecretStore, uc: &UserConfig) {
    // Only pass keys where both has_key AND granted
    if uc.anthropic_granted { config.llm.nearai.api_key = store.get("anthropic"); }
    if uc.openai_granted    { config.llm.nearai.openai_key = store.get("openai"); }
    // ... more providers ...
}
```

### 3.2 Tauri Channel (`ironclaw_channel.rs`, ~120 lines)

> **IMPORTANT:** The `session_key` must be populated on every event. The frontend
> filters events by `uiEvent.session_key !== effectiveSessionKey` — events with
> empty session keys are silently dropped and nothing renders. The `TauriChannel`
> tracks the active session via an `Arc<RwLock<String>>` that is set by
> `ironclaw::api::chat::send_message()` before spawning the agent turn.

> **CONCURRENCY NOTE:** The `active_session: Arc<RwLock<String>>` is a single
> global value. If two turns are in-flight on different sessions concurrently,
> the second `send_message()` overwrites `active_session` and the first turn's
> streaming events get tagged with the wrong session. For a single-user desktop
> app this is rare (the UI typically blocks input during a turn), but not
> impossible. **Future IronClaw fix:** propagate session context through
> `StatusUpdate` metadata or add an `Option<String>` session field to
> `StatusUpdate` variants, so `send_status()` can read the session from the
> event itself instead of a shared mutable. Until then, this workaround is
> acceptable.

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ironclaw::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use ironclaw::error::ChannelError;
use tauri::Emitter;
use tokio::sync::RwLock;

use crate::openclaw::normalizer::UiEvent;

pub struct TauriChannel {
    app: tauri::AppHandle,
    /// Tracks the session_key of the currently-active agent turn.
    /// Set by `api::chat::send_message()` before spawning the turn.
    /// Read by `send_status()` and `respond()` to tag events.
    ///
    /// Without this, all streamed events have empty session_key and
    /// the frontend's `if (uiEvent.session_key !== effectiveSessionKey) return;`
    /// filter silently drops every event — nothing renders.
    pub active_session: Arc<RwLock<String>>,
}

impl TauriChannel {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            active_session: Arc::new(RwLock::new(String::new())),
        }
    }

    /// Helper: get the current session key for tagging events.
    async fn session_key(&self) -> String {
        self.active_session.read().await.clone()
    }
}

#[async_trait]
impl Channel for TauriChannel {
    fn name(&self) -> &str { "tauri" }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // Messages arrive via direct API calls, not a stream.
        // Return an idle stream (never produces items).
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    /// Called at the END of an agent turn with the final accumulated response.
    /// This is called once per turn (not per-token — streaming tokens arrive
    /// via `send_status(StatusUpdate::StreamChunk)`).
    async fn respond(&self, msg: &IncomingMessage, response: OutgoingResponse) -> Result<(), ChannelError> {
        // IMPORTANT: Use msg.thread_id (set by IncomingMessage::with_thread())
        // as the authoritative session_key. Fall back to active_session only
        // if thread_id is missing. NEVER use futures::executor::block_on()
        // inside an async fn — it can deadlock the tokio runtime.
        let session_key = match msg.thread_id.clone() {
            Some(tid) => tid,
            None => self.session_key().await,
        };
        let event = UiEvent::AssistantFinal {
            session_key,
            run_id: None,
            message_id: msg.id.to_string(),
            text: response.content,
            usage: None,
        };
        self.app.emit("openclaw-event", &event).ok();
        Ok(())
    }

    async fn send_status(&self, status: StatusUpdate, _metadata: &serde_json::Value) -> Result<(), ChannelError> {
        let sk = self.session_key().await;
        let event = match status {
            StatusUpdate::Thinking(_) => UiEvent::RunStatus {
                session_key: sk, run_id: None,
                status: "started".into(), error: None,
            },
            StatusUpdate::StreamChunk(chunk) => UiEvent::AssistantDelta {
                session_key: sk, run_id: None,
                message_id: "stream".into(), delta: chunk,
            },
            StatusUpdate::ToolStarted { name } => UiEvent::ToolUpdate {
                session_key: sk, run_id: None,
                tool_name: name, status: "started".into(),
                input: serde_json::Value::Null, output: serde_json::Value::Null,
            },
            StatusUpdate::ToolCompleted { name, success } => UiEvent::ToolUpdate {
                session_key: sk, run_id: None,
                tool_name: name, status: if success { "ok" } else { "error" }.into(),
                input: serde_json::Value::Null, output: serde_json::Value::Null,
            },
            StatusUpdate::ApprovalNeeded { request_id, tool_name, description, parameters } => {
                UiEvent::ApprovalRequested {
                    approval_id: request_id,
                    session_key: sk,
                    tool_name,
                    input: parameters,
                }
            },
            // NEW: Turn-level errors emitted by spawned tasks in api::chat
            StatusUpdate::Error { message, code } => UiEvent::Error {
                code: code.unwrap_or_else(|| "turn_error".into()),
                message,
                details: serde_json::Value::Null,
            },
            _ => return Ok(()),
        };
        self.app.emit("openclaw-event", &event).ok();
        Ok(())
    }

    /// Desktop is single-user — broadcast is for system-originated messages
    /// (self-repair notifications, routine results, heartbeat).
    /// Maps to AssistantInternal (not AssistantFinal) so the frontend can
    /// distinguish agent-initiated system messages from user-turn responses.
    async fn broadcast(&self, _user_id: &str, response: OutgoingResponse) -> Result<(), ChannelError> {
        let sk = self.session_key().await;
        let event = UiEvent::AssistantInternal {
            session_key: sk,
            run_id: None,
            message_id: uuid::Uuid::new_v4().to_string(),
            text: response.content,
        };
        self.app.emit("openclaw-event", &event).ok();
        Ok(())
    }

    /// Required by the Channel trait. Desktop channel is always healthy.
    async fn health_check(&self) -> Result<(), ChannelError> { Ok(()) }
}
```

### 3.3 Rewritten Commands (Example Before/After)

**Before (WebSocket RPC — blocks on WS round-trip):**
```rust
#[tauri::command]
pub async fn openclaw_send_message(
    state: State<'_, OpenClawManager>,
    session_key: String, text: String, deliver: bool,
) -> Result<OpenClawRpcResponse, String> {
    let handle = state.ws_handle.lock().await;
    let handle = handle.as_ref().ok_or("Not connected")?;
    handle.chat_send(&session_key, &uuid::Uuid::new_v4().to_string(), &text, deliver)
        .await.map_err(|e| e.to_string())
}
```

**After (Hybrid API — returns instantly, streams via events):**
```rust
#[tauri::command]
pub async fn openclaw_send_message(
    state: State<'_, IronClawState>,
    session_key: String, text: String, deliver: bool,
) -> Result<serde_json::Value, String> {
    // CRITICAL: Set the active session on TauriChannel BEFORE spawning
    // the agent turn. Without this, all streamed events have empty
    // session_key and the frontend silently drops them.
    *state.channel.active_session.write().await = session_key.clone();

    // Returns in ~5ms. Agent turn runs in background.
    // Response streams back via app.emit("openclaw-event") events.
    let result = ironclaw::api::chat::send_message(
        &state.agent, &session_key, &text, deliver,
    ).await.map_err(|e| e.to_string())?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

// Compare with a QUERY command (truly synchronous):
#[tauri::command]
pub async fn openclaw_list_sessions(
    state: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    // Returns in ~1ms. No background task — direct data read.
    let sessions = ironclaw::api::sessions::list_sessions(
        &state.components.session_manager,
        state.components.db.as_deref(),
        "local_user",
    ).await.map_err(|e| e.to_string())?;
    serde_json::to_value(sessions).map_err(|e| e.to_string())
}
```

### 3.4 `lib.rs` Setup Changes

```rust
// In Tauri setup, after existing state registration:

let secret_store_ref = handle.state::<crate::secret_store::SecretStore>();
let user_config_ref = handle.state::<crate::config::UserConfig>();

match crate::ironclaw_bridge::init_ironclaw(&handle, &secret_store_ref, &user_config_ref).await {
    Ok(ironclaw_state) => {
        handle.manage(ironclaw_state);
        handle.emit("openclaw-event", UiEvent::Connected { protocol: 1 }).ok();
    }
    Err(e) => {
        handle.emit("openclaw-event", UiEvent::Error {
            code: "init_failed".into(),
            message: e.to_string(),
            details: serde_json::Value::Null,
        }).ok();
    }
}

// REMOVE: OpenClawManager::new(), Node.js sidecar spawn, WS handle storage
```

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

IronClaw's `Submission::UserInput` always triggers a turn today. **New method
`Agent::inject_context()` needed** (see §2.2).

### 5.2 Silent Reply Handling (`NO_REPLY` / `NO_REPL`)

IronClaw uses `NO_REPLY` token (in `llm/reasoning.rs`). Scrappy uses `NO_REPL`.
The agent loop already suppresses these at line 720 of `agent_loop.rs`. For Tauri,
IronClaw should **emit raw text** and let Scrappy apply its own sanitization
(tuned for its local model zoo).

### 5.3 Boot Sequence Protocol

On OpenClaw tab open, Scrappy sends a `SYSTEM_BOOT_SEQUENCE` message composed from
workspace files (SOUL.md, MEMORY.md, BOOTSTRAP.md). This is just a `chat.send()`
with `deliver=true`. IronClaw processes it like any other user input.

### 5.4 Token Sanitization

Scrappy's `normalizer.rs` has 10 compiled regexes for stripping ChatML/Jinja
tokens. This stays in Scrappy. IronClaw emits raw LLM output — Scrappy sanitizes
before displaying.

### 5.5 Security: Zeroize on Drop

Scrappy's `OpenClawConfig` implements `Drop` with `zeroize()` on API key fields.
IronClaw uses `secrecy::SecretString` which zeroizes on drop. Compatible approaches.

### 5.6 Model Selection & Fallback

IronClaw's `build_provider_chain()` implements: primary → retry → smart routing →
failover → circuit breaker → cache. This maps to Scrappy's model resolution:
1. Scrappy resolves primary + fallback models from settings + grants
2. Passes result into IronClaw's `Config` before calling `AppBuilder`
3. IronClaw builds the chain from config (it does NOT auto-discover models)

### 5.7 Specta TypeScript Bindings

Scrappy uses `tauri-specta` for TypeScript binding generation. All Tauri command
types must be `specta::Type`. IronClaw types should be **wrapped in Scrappy-side
DTOs** that derive `specta::Type`, not add `specta` as a dependency to IronClaw.

### 5.8 Rust Edition

IronClaw uses Rust edition **2024** (MSRV 1.92). Scrappy uses edition **2021**.
This is not a blocker (editions are per-crate in a workspace), but Scrappy's
toolchain must support 1.92+.

### 5.9 MCP Reverse-RPC

With IronClaw in-process, the MCP reverse-RPC is eliminated. IronClaw calls tools
directly via `ToolRegistry`. If IronClaw needs Scrappy-hosted tools (browser,
local file access), it should expose a trait/callback mechanism.

---

## 6. Database Strategy

| Database | Engine | Content | Managed By |
|---|---|---|---|
| `openclaw.db` | SQLite (sqlx) | Chat tab conversations, documents, vectors, images | Scrappy |
| `ironclaw.db` | libSQL | OpenClaw tab sessions, memory, workspace, cron, skills | IronClaw |

Separate databases serving different domains. Do NOT unify initially.

---

## 7. New Code Summary

### IronClaw Side (Build/Refactor)

| Change | Est. Lines | Complexity |
|---|---|---|
| `ironclaw::api` module (chat, sessions, memory, config, extensions, skills, routines, system) | ~500 | Medium — extract from handlers |
| Make `handle_message()` public + add `inject_context()` + `cancel_turn()` | ~50 | Low |
| Extract `start_background_tasks()` from `run()` + add `shutdown()` | ~60 | Medium |
| Add `StatusUpdate::Error` variant to `channels/channel.rs` | ~5 | Low |
| Accept external secrets/keys via Config | ~20 | Low |
| Feature flags (Cargo.toml + `#[cfg]` in lib.rs) | ~50 | Low |
| `EngineStatus` struct for system status | ~80 | Low |
| **Total IronClaw changes** | **~765** | |

### Scrappy Side (Build/Adapt)

| Change | Est. Lines | Complexity |
|---|---|---|
| `ironclaw_bridge.rs` (engine init) | ~100 | Low |
| `ironclaw_channel.rs` (TauriChannel) | ~80 | Low |
| Rewrite 50+ commands from WS → direct API | ~500 | Medium (tedious) |
| `lib.rs` modifications | ~40 | Low |
| Delete WS bridge files | -90 KB | Low |
| **Total Scrappy changes** | **~720 new, ~90 KB deleted** | |

---

## 8. Implementation Checklist

### Phase 1: IronClaw Library Prep (2-3 days)

- [ ] Add `StatusUpdate::Error { message, code }` variant to `channels/channel.rs`
- [ ] Add `ironclaw::api` module with extracted handler logic
- [ ] Make `Agent::handle_message()` public as `handle_message_external()`
- [ ] Add `Agent::inject_context()` for deliver=false messages
- [ ] Add `Agent::cancel_turn()` for direct abort (bypasses hooks)
- [ ] Extract `start_background_tasks()` and `shutdown()` from `Agent::run()`
- [ ] Add feature flags to Cargo.toml (`desktop`, `web-gateway`, `full`)
- [ ] Gate modules with `#[cfg(feature)]` in lib.rs
- [ ] Verify `cargo build --lib --no-default-features --features desktop` compiles
- [ ] Ensure `AppBuilder` accepts keys via Config fields
- [ ] Create `EngineStatus` struct
- [ ] Export all needed types from lib.rs

### Phase 2: Scrappy Integration Scaffold (1-2 days)

- [ ] Add IronClaw as dependency: `ironclaw = { path = "../ironclaw", features = ["desktop"] }`
- [ ] Create `ironclaw_bridge.rs` with `init_ironclaw()` calling `AppBuilder::build_all()`
- [ ] Create `ironclaw_channel.rs` with `TauriChannel` implementing `Channel` trait
- [ ] Register `IronClawState` as Tauri managed state in `lib.rs`
- [ ] Wire up event emission from `TauriChannel`
- [ ] Verify Tauri builds with IronClaw dependency

### Phase 3: Command Migration (2-3 days)

- [ ] Rewrite `openclaw/commands/sessions.rs` (12 commands)
- [ ] Rewrite `openclaw/commands/rpc.rs` (skills, cron, config, system commands)
- [ ] Rewrite `openclaw/commands/gateway.rs` (simplified init)
- [ ] Adapt `openclaw/commands/keys.rs` (SecretStore → Config mapping)
- [ ] Rewrite `openclaw/fleet.rs` (or stub for now)
- [ ] Test: send message from OpenClaw tab → streaming response renders

### Phase 4: Cleanup (1 day)

- [ ] Delete `openclaw/ws_client.rs`, `normalizer.rs`, `frames.rs`
- [ ] Extract token sanitizer to small utility (keep regex patterns)
- [ ] Refactor `openclaw/ipc.rs` → remove WS reverse-RPC
- [ ] Remove `OpenClawManager` struct
- [ ] Remove Node.js sidecar from `sidecar.rs`
- [ ] Remove `node` from `tauri.conf.json` external bins
- [ ] Remove `openclaw-engine/` directory and npm scripts

### Phase 5: Verification (1 day)

- [ ] All 50+ `openclaw_*` commands work
- [ ] Approval flow end-to-end
- [ ] Skills / cron / extensions
- [ ] Frontend receives all `UiEvent` variants
- [ ] Boot sequence works (SOUL.md → MEMORY.md → BOOTSTRAP.md)
- [ ] Latency comparison vs. old WS bridge

**Total estimated time: 7-10 days**

---

## 9. Critical Notes for the IronClaw Agent

1. **You are a library, not the app.** IronClaw replaces only the OpenClaw
   Node.js engine — everything else in Scrappy stays.

2. **The `ironclaw::api` module is the highest-impact deliverable.** Without it,
   Scrappy must either duplicate business logic or use the Channel-Feed approach
   (worse UX).

3. **`Agent::handle_message_external()` is the key unlock.** Making
   `handle_message()` callable from outside the `agent` module is what enables
   the entire Hybrid API architecture.

4. **Agent turns are spawned, NOT awaited.** `send_message()` and
   `resolve_approval()` must use `tokio::spawn()` and return immediately.
   Only queries and simple writes are awaited. This is the single most
   important UX decision — blocking the Tauri command for 10-30s makes the
   UI appear frozen.

5. **Feature flags, not deletion.** Keep web-gateway, REPL, tunnel, Docker modules
   behind feature flags.

6. **Accept keys via Config strings.** Let Scrappy resolve Keychain → grant
   flags → Config. IronClaw should not access Scrappy's Keychain.

7. **Accept external inference URLs.** Scrappy manages local llama-server/MLX/vLLM.
   IronClaw points at `http://127.0.0.1:{port}/v1`.

8. **The UiEvent contract is sacred.** Frontend React components already listen
   for specific shapes. IronClaw events must map to these.

9. **Two databases is correct.** Chat SQLite (Scrappy) + agent libSQL (IronClaw).

10. **LLM token sanitization stays in Scrappy.** Emit raw text. Scrappy sanitizes.

11. **Background tasks need graceful shutdown.** Extract from `Agent::run()` so
    Tauri can stop them on app quit.

---

## 10. Critical Notes for the Scrappy Agent

1. **Don't await agent turns.** The `send_message` and `resolve_approval` API
   functions spawn background tasks. The Tauri command returns `Ok(message_id)`
   in ~5ms. Response content arrives as `"openclaw-event"` emissions.

2. **Error handling is split.** Validation errors (empty message, bad UUID,
   missing config) come back as `Err()` from the Tauri command. Turn errors
   (LLM down, tool crash, safety rejection) come as `UiEvent::Error` events.
   Both paths must surface in the UI.

3. **`IronClawState.agent` must be `Arc<Agent>`.** The spawn pattern requires
   cloning the Arc into the spawned future. All Tauri commands receive the
   Agent via `State<'_, IronClawState>` which holds `Arc<Agent>`.

4. **Token sanitization stays in Scrappy.** IronClaw emits raw LLM output.
   Apply your regex sanitizers before rendering in the UI.

5. **Boot sequence is a normal `send_message()`.** Compose
   `SYSTEM_BOOT_SEQUENCE` from workspace files, call `send_message` with
   `deliver=true`. IronClaw processes it like any other user input.

6. **Secrets flow one way.** Read from Keychain → check grant flags →
   inject into `ironclaw::Config` → pass to `AppBuilder`. IronClaw never
   reads Scrappy's Keychain directly.

7. **Specta types are Scrappy-side DTOs.** Wrap IronClaw return types in
   Scrappy structs that derive `specta::Type`. Don't add specta as a dep
   to IronClaw.

8. **Inference URL is Scrappy's responsibility.** Resolve which sidecar is
   running, get its port, pass `http://127.0.0.1:{port}/v1` into the
   IronClaw config before init.

9. **Shutdown on app quit.** Call `agent.shutdown(handle)` in the Tauri
   `on_window_event(CloseRequested)` handler to stop background tasks.

10. **The `deliver` flag matters.** `deliver=false` adds to history silently
    (for context injection). `deliver=true` triggers a full agent turn.
    Boot sequences use `deliver=true`. Date context uses `deliver=false`.
