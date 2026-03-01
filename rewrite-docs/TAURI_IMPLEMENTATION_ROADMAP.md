# ThinClaw Tauri Integration — Implementation Roadmap

> **Status:** ✅ **COMPLETE** — All phases finished 2026-02-28 (~1 day actual, vs 7-10 day estimate)
> **Derived from:** `TAURI_INTEGRATION.md` (v2, 2026-02-27)
> **Total estimated effort:** 7–10 working days · **Actual:** ~1 day (parallel agent execution)
> **Tracks:** IronClaw (library prep) → Scrappy (integration) → Joint (verification)
> **Ordering rule:** Each step lists its dependencies. Never start a step before
> its dependencies are checked off.
>
> _This document is now historical. For completion details see `TAURI_INTEGRATION.md` § Implementation Progress._

---

## Track Overview

```
Week 1                                          Week 2
┌───────────────────────────────────────┐  ┌────────────────────────────────┐
│  IRONCLAW (library prep)              │  │  SCRAPPY (integration)         │
│                                       │  │                                │
│  Phase 1: Core Plumbing (Day 1)       │  │  Phase 3: Scaffold (Day 4-5)   │
│  Phase 2: API Surface (Day 2-3)       │  │  Phase 4: Commands (Day 5-7)   │
│                                       │  │  Phase 5: Cleanup (Day 8)      │
│  ─── Gate: IronClaw compiles ──────── │  │  Phase 6: Verify (Day 9-10)    │
│       with `--features desktop`       │  │                                │
└───────────────────────────────────────┘  └────────────────────────────────┘
```

Both tracks can **overlap starting at Phase 3** — once IronClaw's Phase 1 is
done (types compile), Scrappy can scaffold while IronClaw finishes Phase 2.

---

## Phase 1: IronClaw Core Plumbing (Day 1)

**Goal:** Make the crate compilable as a library with feature flags, expose
the minimum types needed for Scrappy to start scaffolding.

### Step 1.1 — Add `StatusUpdate::Error` variant

**File:** `src/channels/channel.rs`
**Side:** IronClaw
**Depends on:** Nothing
**Effort:** 15 min

Add to the `StatusUpdate` enum:

```rust
/// Turn-level error surfaced to the UI (e.g., LLM unreachable, safety reject).
Error {
    message: String,
    code: Option<String>,
},
```

**Acceptance:** `cargo check` passes. No consumers need updating yet (the
`_` wildcard arm in existing `match` statements handles it).

---

### Step 1.2 — Add feature flags to `Cargo.toml`

**File:** `Cargo.toml`
**Side:** IronClaw
**Depends on:** Nothing
**Effort:** 30 min

```toml
[features]
default = ["postgres", "libsql", "html-to-markdown"]
desktop = ["libsql", "html-to-markdown"]
web-gateway = []
repl = []
tunnel = []
docker-sandbox = []
full = ["postgres", "libsql", "html-to-markdown", "web-gateway", "repl", "tunnel", "docker-sandbox"]
```

**Do NOT move any code behind `#[cfg]` yet.** This step only declares the flags
so that `--features desktop` is a valid compilation target.

**Acceptance:** `cargo check --features desktop` passes (all features are additive,
no code is gated yet, so this should work immediately).

---

### Step 1.3 — Gate heavy modules behind feature flags

**File:** `src/lib.rs` + affected module files
**Side:** IronClaw
**Depends on:** Step 1.2
**Effort:** 1-2 hours

Apply `#[cfg(feature = "...")]` to non-essential modules:

```rust
// src/lib.rs — conditional compilation
#[cfg(feature = "web-gateway")]
pub mod channels_web_server; // or gate within channels/web/server.rs

#[cfg(feature = "repl")]
pub mod channels_repl;       // or gate channels/repl.rs

#[cfg(feature = "tunnel")]
pub mod tunnel;

#[cfg(feature = "docker-sandbox")]
pub mod orchestrator;
#[cfg(feature = "docker-sandbox")]
pub mod worker;
```

**Critical subtasks:**
1. Identify all `use` statements that reference gated modules — wrap them too
2. Move `LogBroadcaster` from `channels/web/log_layer.rs` to a shared location
   (e.g., `src/observability/log_broadcaster.rs`) since it's needed by `AppBuilder`
   regardless of feature flags
3. Ensure `channels/channel.rs` (the trait) and `channels/web/types.rs` (DTOs)
   remain unconditional — they're needed by all modes

**Acceptance:** Both compile:
- `cargo check --no-default-features --features desktop`
- `cargo check --features full`

---

### Step 1.4 — Make `Agent::handle_message()` public

**File:** `src/agent/agent_loop.rs`
**Side:** IronClaw
**Depends on:** Nothing
**Effort:** 15 min

```rust
// Change visibility: pub(super) → pub
pub async fn handle_message_external(
    &self,
    message: &IncomingMessage,
) -> Result<Option<String>, Error> {
    self.handle_message(message).await
}
```

This is a one-line wrapper. Keep the original `handle_message()` as `pub(super)`
for internal use.

**Acceptance:** `cargo check` passes. Method is accessible from outside `agent` module.

---

### Step 1.5 — Add `Agent::inject_context()`

**File:** `src/agent/agent_loop.rs`
**Side:** IronClaw
**Depends on:** Step 1.4
**Effort:** 20 min

```rust
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
```

**Acceptance:** Can call `agent.inject_context(msg).await` from outside the module.
Message appears in session history without triggering a turn.

---

### Step 1.6 — Add `Agent::cancel_turn()`

**File:** `src/agent/agent_loop.rs`
**Side:** IronClaw
**Depends on:** Step 1.4
**Effort:** 15 min

```rust
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
```

**Acceptance:** Calling `cancel_turn` on an active turn sets the interrupt flag.
The running LLM call or tool execution stops at its next yield point.

---

### Step 1.7 — Extract background tasks from `Agent::run()`

**File:** `src/agent/agent_loop.rs`
**Side:** IronClaw
**Depends on:** Steps 1.4–1.6
**Effort:** 1-2 hours

**Subtasks:**

1. Create `BackgroundTasksHandle` struct:
   ```rust
   pub struct BackgroundTasksHandle {
       shutdown_tx: tokio::sync::watch::Sender<bool>,
       handles: Vec<tokio::task::JoinHandle<()>>,
   }
   ```

2. Extract `start_background_tasks(&self) -> BackgroundTasksHandle`:
   - Move self-repair spawning (lines ~228-307 of current `run()`)
   - Move session pruning spawning (lines ~310-319)
   - Move heartbeat spawning (lines ~322-389)
   - Move routine engine spawning (lines ~392-466)
   - Each spawned task listens to `shutdown_rx` for graceful stop

3. Add `shutdown(&self, handle: BackgroundTasksHandle)`:
   ```rust
   pub async fn shutdown(&self, handle: BackgroundTasksHandle) {
       handle.shutdown_tx.send(true).ok();
       for h in handle.handles {
           h.abort();
       }
       if let Some(scheduler) = &self.scheduler {
           scheduler.stop_all().await;
       }
   }
   ```

4. Refactor existing `run()` to use the new methods internally:
   ```rust
   pub async fn run(self) -> Result<(), Error> {
       let bg = self.start_background_tasks();
       let mut message_stream = self.channels.start_all().await?;
       loop { /* existing message loop */ }
       self.shutdown(bg).await;
       Ok(())
   }
   ```

**Acceptance:**
- CLI/REPL mode still works via `agent.run()` (no behavior change)
- Can call `start_background_tasks()` and `shutdown()` independently
- Background tasks stop within 1s of `shutdown()` being called

---

### Step 1.8 — Ensure `AppBuilder` accepts keys via Config

**File:** `src/app.rs`, `src/config/mod.rs`
**Side:** IronClaw
**Depends on:** Nothing
**Effort:** 30 min

Verify that `Config` struct fields accept API keys directly (they likely
already do). Ensure `init_secrets()` can be skipped when keys are pre-populated:

```rust
impl AppBuilder {
    pub fn with_secrets_store(
        mut self,
        store: Arc<dyn SecretsStore + Send + Sync>,
    ) -> Self {
        self.external_secrets = Some(store);
        self
    }
}
```

In `init_secrets()`:
```rust
if let Some(external) = self.external_secrets.take() {
    self.secrets_store = external;
    return Ok(());
}
```

**Acceptance:** Can build an `AppComponents` with pre-provided API keys
without touching the system keychain.

---

### Step 1.9 — Export types from `lib.rs`

**File:** `src/lib.rs`
**Side:** IronClaw
**Depends on:** Steps 1.1–1.8
**Effort:** 15 min

```rust
pub mod api;  // NEW (empty placeholder for now — filled in Phase 2)

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

**Acceptance:**
- `cargo check --no-default-features --features desktop` succeeds
- External crate can `use ironclaw::prelude::*` and access all needed types

---

### ⬤ Phase 1 Gate

**Milestone:** IronClaw compiles as a library with `--features desktop`.
All types Scrappy needs are public. No API module logic yet, but the
placeholder exists. **Scrappy can begin Phase 3 (scaffolding) in parallel
with Phase 2.**

---

## Phase 2: IronClaw API Surface (Days 2-3)

**Goal:** Build the `ironclaw::api` module — framework-agnostic functions
that expose all agent capabilities as callable Rust functions.

### Step 2.1 — Create `api/mod.rs` with module structure

**File:** `src/api/mod.rs`
**Side:** IronClaw
**Depends on:** Phase 1 complete
**Effort:** 10 min

```rust
pub mod chat;
pub mod sessions;
pub mod memory;
pub mod config;
pub mod extensions;
pub mod skills;
pub mod routines;
pub mod system;

mod error;
pub use error::ApiError;
```

---

### Step 2.2 — Create `api/error.rs`

**File:** `src/api/error.rs`
**Side:** IronClaw
**Depends on:** Step 2.1
**Effort:** 30 min

```rust
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Agent error: {0}")]
    Agent(#[from] crate::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("UUID parse error: {0}")]
    UuidParse(#[from] uuid::Error),
}

impl ApiError {
    pub fn error_code(&self) -> String {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::SessionNotFound(_) => "session_not_found",
            Self::Agent(_) => "agent_error",
            Self::Serialization(_) => "serialization_error",
            Self::UuidParse(_) => "uuid_parse_error",
        }.into()
    }
}
```

---

### Step 2.3 — Implement `api/chat.rs` (critical path)

**File:** `src/api/chat.rs`
**Side:** IronClaw
**Depends on:** Steps 1.1, 1.4, 1.5, 1.6, 2.2
**Effort:** 1-2 hours

Functions:
- `send_message(agent, session_key, message, deliver) -> Result<SendMessageResult>`
  — Validates, spawns turn via `tokio::spawn`, emits `StatusUpdate::Error` on failure
- `resolve_approval(agent, session_key, approval_id, approved, always) -> Result<()>`
  — Spawns approval processing as background task
- `abort(agent, session_key) -> Result<()>`
  — Calls `agent.cancel_turn()` directly

**Source logic:** Extract from `channels/web/handlers/chat.rs` (`chat_send_handler`,
`chat_approve_handler`, `chat_abort_handler`). Strip Axum wrappers, keep business logic.

**Acceptance:**
- `send_message` returns in <10ms (doesn't block on LLM)
- Failed turns emit `StatusUpdate::Error` through channel infrastructure
- `abort` interrupts a running turn within 100ms

---

### Step 2.4 — Implement `api/sessions.rs`

**File:** `src/api/sessions.rs`
**Side:** IronClaw
**Depends on:** Step 2.2
**Effort:** 1-2 hours

Functions:
- `list_sessions(session_manager, db, user_id) -> Result<Vec<SessionInfo>>`
- `get_history(session_manager, db, session_key, limit, before) -> Result<HistoryResponse>`
- `create_session(session_manager, user_id) -> Result<SessionInfo>`
- `delete_session(session_manager, db, session_key) -> Result<()>`
- `clear_session(session_manager, session_key) -> Result<()>`

**Source logic:** Extract from `channels/web/handlers/sessions.rs` and
`agent/session_manager.rs` public methods.

**Acceptance:** Each function returns typed data without requiring Axum state.

---

### Step 2.5 — Implement `api/memory.rs`

**File:** `src/api/memory.rs`
**Side:** IronClaw
**Depends on:** Step 2.2
**Effort:** 1 hour

Functions:
- `get_file(workspace, path) -> Result<String>`
- `write_file(workspace, path, content) -> Result<()>`
- `list_files(workspace) -> Result<Vec<FileEntry>>`
- `clear(workspace, target) -> Result<()>`
- `search(workspace, query, limit) -> Result<Vec<SearchResult>>`

**Source logic:** Thin wrappers around `Workspace` methods.

---

### Step 2.6 — Implement `api/config.rs`

**File:** `src/api/config.rs`
**Side:** IronClaw
**Depends on:** Step 2.2
**Effort:** 30 min

Functions:
- `get_config(components) -> ConfigSnapshot`
- `set_config(components, key, value) -> Result<()>`

---

### Step 2.7 — Implement `api/skills.rs`

**File:** `src/api/skills.rs`
**Side:** IronClaw
**Depends on:** Step 2.2
**Effort:** 1 hour

Functions:
- `list_skills(skill_registry) -> Result<Vec<SkillInfo>>`
- `toggle_skill(skill_registry, skill_id, enabled) -> Result<()>`
- `install_skill(skill_catalog, url_or_name) -> Result<SkillInfo>`

---

### Step 2.8 — Implement `api/routines.rs`

**File:** `src/api/routines.rs`
**Side:** IronClaw
**Depends on:** Step 2.2
**Effort:** 1 hour

Functions:
- `list_routines(routine_engine) -> Result<Vec<RoutineInfo>>`
- `trigger_routine(routine_engine, routine_id) -> Result<()>`
  — This is a 🔥 Spawn operation (background task)

---

### Step 2.9 — Implement `api/system.rs`

**File:** `src/api/system.rs`
**Side:** IronClaw
**Depends on:** Step 2.2
**Effort:** 1 hour

Functions:
- `get_status(components) -> EngineStatus` — synchronous struct read
- `health_check() -> HealthStatus`
- `list_models(llm) -> Result<Vec<ModelInfo>>`
- `tail_logs(broadcaster, limit) -> Vec<LogEntry>`
- `diagnostics(components) -> DiagnosticsReport`

Create `EngineStatus` struct:
```rust
#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub engine_running: bool,
    pub setup_completed: bool,
    pub tool_count: usize,
    pub active_extensions: usize,
    pub model_name: Option<String>,
    pub db_connected: bool,
    pub uptime_seconds: u64,
}
```

---

### Step 2.10 — Implement `api/extensions.rs`

**File:** `src/api/extensions.rs`
**Side:** IronClaw
**Depends on:** Step 2.2
**Effort:** 30 min

Functions:
- `list_extensions(extension_manager) -> Result<Vec<ExtensionInfo>>`
- `toggle_extension(extension_manager, ext_id, enabled) -> Result<()>`

---

### ⬤ Phase 2 Gate

**Milestone:** `ironclaw::api` is complete. All functions compile and are
callable from external crates. Run:
```
cargo test --features desktop
cargo doc --features desktop --no-deps
```
Verify docs render for all public API functions.

---

## Phase 3: Scrappy Integration Scaffold (Days 4-5)

**Goal:** Get IronClaw running inside Scrappy's Tauri process. First
successful event emitted to the frontend.

> **NOTE:** This phase can start as soon as Phase 1 completes. Phase 2
> can continue in parallel.

### Step 3.1 — Add IronClaw as dependency

**File:** `src-tauri/Cargo.toml`
**Side:** Scrappy
**Depends on:** IronClaw Phase 1 complete
**Effort:** 15 min

```toml
[dependencies]
ironclaw = { path = "../../ironclaw/ironclaw", default-features = false, features = ["desktop"] }
```

Ensure Scrappy's Rust toolchain supports edition 2024 / MSRV 1.92+:
```bash
rustup update
rustc --version  # must be >= 1.92
```

**Acceptance:** `cargo check` in `src-tauri/` succeeds (even if nothing
uses IronClaw yet).

---

### Step 3.2 — Create `ironclaw_channel.rs`

**File:** `src-tauri/src/openclaw/ironclaw_channel.rs`
**Side:** Scrappy
**Depends on:** Step 3.1
**Effort:** 1-2 hours

Implement the `Channel` trait for Tauri:

**Key implementation details:**
- `active_session: Arc<RwLock<String>>` for session tracking
- `respond()` uses `msg.thread_id` as authoritative session_key, falls back to `active_session`
- `send_status()` maps all `StatusUpdate` variants → `UiEvent` variants
  (including `StatusUpdate::Error` → `UiEvent::Error`)
- `broadcast()` maps to `UiEvent::AssistantInternal`
- `health_check()` returns `Ok(())`
- `start()` returns an idle stream (messages arrive via direct API)

**Acceptance:** `cargo check` passes. Channel struct can be instantiated
with a `tauri::AppHandle`.

---

### Step 3.3 — Create `ironclaw_bridge.rs`

**File:** `src-tauri/src/openclaw/ironclaw_bridge.rs`
**Side:** Scrappy
**Depends on:** Steps 3.1, 3.2
**Effort:** 2-3 hours

Implement:

1. `IronClawState` struct:
   ```rust
   pub struct IronClawState {
       pub components: Arc<AppComponents>,
       pub agent: Arc<Agent>,
       pub channel: Arc<TauriChannel>,
       pub background: tokio::sync::Mutex<Option<BackgroundTasksHandle>>,
   }
   ```

2. `init_ironclaw(app, secret_store, user_config) -> Result<IronClawState>`:
   - Build `Config` from Scrappy settings
   - Inject granted API keys from `SecretStore`
   - Set inference URL from sidecar port
   - Call `AppBuilder::build_all()`
   - Create `TauriChannel`, register with `ChannelManager` (`.clone()` first!)
   - Create `Agent`, start background tasks
   - Return `IronClawState`

3. `inject_granted_keys()` helper

4. `shutdown_ironclaw(state)`:
   ```rust
   pub async fn shutdown_ironclaw(state: &IronClawState) {
       if let Some(handle) = state.background.lock().await.take() {
           state.agent.shutdown(handle).await;
       }
   }
   ```

**Acceptance:** `init_ironclaw()` runs without panic, returns `IronClawState`.
Not wired to Tauri yet.

---

### Step 3.4 — Wire IronClaw into Tauri `setup()`

**File:** `src-tauri/src/lib.rs`
**Side:** Scrappy
**Depends on:** Step 3.3
**Effort:** 1 hour

In the Tauri builder's `.setup()` handler:

```rust
// After existing state registration (SecretStore, UserConfig, etc.)
let secret_store_ref = handle.state::<SecretStore>();
let user_config_ref = handle.state::<UserConfig>();

match ironclaw_bridge::init_ironclaw(&handle, &secret_store_ref, &user_config_ref).await {
    Ok(ironclaw_state) => {
        handle.manage(ironclaw_state);
        handle.emit("openclaw-event", UiEvent::Connected { protocol: 1 }).ok();
    }
    Err(e) => {
        tracing::error!("IronClaw init failed: {}", e);
        handle.emit("openclaw-event", UiEvent::Error {
            code: "init_failed".into(),
            message: e.to_string(),
            details: serde_json::Value::Null,
        }).ok();
    }
}
```

Add shutdown handler:
```rust
.on_window_event(|window, event| {
    if let tauri::WindowEvent::CloseRequested { .. } = event {
        let state = window.state::<IronClawState>();
        tauri::async_runtime::block_on(ironclaw_bridge::shutdown_ironclaw(&state));
    }
})
```

**Acceptance:** App launches. Frontend receives `UiEvent::Connected`.
Console shows IronClaw initialization logs. App closes cleanly
(background tasks stop within 1s).

---

### Step 3.5 — First end-to-end message

**File:** Create a minimal test command
**Side:** Scrappy
**Depends on:** Steps 3.4 + IronClaw Phase 2 Step 2.3
**Effort:** 1 hour

```rust
#[tauri::command]
pub async fn openclaw_send_message(
    state: State<'_, IronClawState>,
    session_key: String, text: String, deliver: bool,
) -> Result<serde_json::Value, String> {
    *state.channel.active_session.write().await = session_key.clone();
    let result = ironclaw::api::chat::send_message(
        &state.agent, &session_key, &text, deliver,
    ).await.map_err(|e| e.to_string())?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}
```

Register it and test from the frontend.

**Acceptance:** Type a message in the OpenClaw tab → see streaming
response appear token by token → response completes with AssistantFinal.

---

### ⬤ Phase 3 Gate

**Milestone:** IronClaw runs in-process. First message round-trips
successfully. Streaming works. Errors surface in the UI.

---

## Phase 4: Command Migration (Days 5-7)

**Goal:** Rewrite all 50+ `openclaw_*` Tauri commands from WebSocket RPC
to direct IronClaw API calls.

### Step 4.1 — Session commands (12 commands)

**File:** `src-tauri/src/openclaw/commands/sessions.rs`
**Side:** Scrappy
**Depends on:** Phase 3 Gate + IronClaw Step 2.4
**Effort:** 2-3 hours

| Command | Maps to |
|---|---|
| `openclaw_list_sessions` | `api::sessions::list_sessions()` |
| `openclaw_get_history` | `api::sessions::get_history()` |
| `openclaw_create_session` | `api::sessions::create_session()` |
| `openclaw_delete_session` | `api::sessions::delete_session()` |
| `openclaw_reset_session` | `api::sessions::clear_session()` |
| `openclaw_rename_session` | `api::sessions::rename_session()` |
| `openclaw_switch_session` | Set `active_session` + `get_history()` |
| `openclaw_undo` | `api::chat::send_message("/undo")` |
| `openclaw_redo` | `api::chat::send_message("/redo")` |
| `openclaw_export_session` | `api::sessions::export()` |
| `openclaw_import_session` | `api::sessions::import()` |
| `openclaw_session_stats` | `api::sessions::stats()` |

---

### Step 4.2 — Chat commands (5 commands)

**File:** `src-tauri/src/openclaw/commands/rpc.rs` (or new file)
**Side:** Scrappy
**Depends on:** Phase 3 Gate + IronClaw Step 2.3
**Effort:** 1 hour

| Command | Maps to |
|---|---|
| `openclaw_send_message` | `api::chat::send_message()` |
| `openclaw_abort` | `api::chat::abort()` |
| `openclaw_approve` | `api::chat::resolve_approval()` |
| `openclaw_reject` | `api::chat::resolve_approval(approved: false)` |
| `openclaw_resend` | Delete last + `send_message()` |

---

### Step 4.3 — Memory/workspace commands (8 commands)

**File:** `src-tauri/src/openclaw/commands/rpc.rs`
**Side:** Scrappy
**Depends on:** Phase 3 Gate + IronClaw Step 2.5
**Effort:** 1-2 hours

| Command | Maps to |
|---|---|
| `openclaw_get_memory` | `api::memory::get_file("MEMORY.md")` |
| `openclaw_save_memory` | `api::memory::write_file("MEMORY.md")` |
| `openclaw_get_soul` | `api::memory::get_file("SOUL.md")` |
| `openclaw_save_soul` | `api::memory::write_file("SOUL.md")` |
| `openclaw_clear_memory` | `api::memory::clear(ClearTarget::Memory)` |
| `openclaw_list_workspace` | `api::memory::list_files()` |
| `openclaw_read_workspace_file` | `api::memory::get_file()` |
| `openclaw_write_workspace_file` | `api::memory::write_file()` |

---

### Step 4.4 — Skills & extensions commands (6 commands)

**File:** `src-tauri/src/openclaw/commands/rpc.rs`
**Side:** Scrappy
**Depends on:** Phase 3 Gate + IronClaw Steps 2.7, 2.10
**Effort:** 1 hour

| Command | Maps to |
|---|---|
| `openclaw_list_skills` | `api::skills::list_skills()` |
| `openclaw_install_skill` | `api::skills::install_skill()` |
| `openclaw_toggle_skill` | `api::skills::toggle_skill()` |
| `openclaw_list_extensions` | `api::extensions::list_extensions()` |
| `openclaw_toggle_extension` | `api::extensions::toggle_extension()` |
| `openclaw_auth_extension` | `api::extensions::auth()` |

---

### Step 4.5 — Routines/cron commands (4 commands)

**File:** `src-tauri/src/openclaw/commands/rpc.rs`
**Side:** Scrappy
**Depends on:** Phase 3 Gate + IronClaw Step 2.8
**Effort:** 30 min

| Command | Maps to |
|---|---|
| `openclaw_list_routines` | `api::routines::list_routines()` |
| `openclaw_trigger_routine` | `api::routines::trigger_routine()` |
| `openclaw_create_routine` | `api::routines::create()` |
| `openclaw_delete_routine` | `api::routines::delete()` |

---

### Step 4.6 — Config & system commands (8 commands)

**File:** `src-tauri/src/openclaw/commands/rpc.rs`
**Side:** Scrappy
**Depends on:** Phase 3 Gate + IronClaw Steps 2.6, 2.9
**Effort:** 1 hour

| Command | Maps to |
|---|---|
| `openclaw_get_config` | `api::config::get_config()` |
| `openclaw_set_config` | `api::config::set_config()` |
| `openclaw_status` | `api::system::get_status()` |
| `openclaw_health` | `api::system::health_check()` |
| `openclaw_diagnostics` | `api::system::diagnostics()` |
| `openclaw_tail_logs` | `api::system::tail_logs()` |
| `openclaw_list_models` | `api::system::list_models()` |
| `openclaw_version` | Return `ironclaw::VERSION` constant |

---

### Step 4.7 — Keys/gateway commands (adapted)

**File:** `src-tauri/src/openclaw/commands/keys.rs`, `gateway.rs`
**Side:** Scrappy
**Depends on:** Phase 3 Gate
**Effort:** 1 hour

These don't call IronClaw API — they manage the bridge:

| Command | Implementation |
|---|---|
| `openclaw_set_key` | `SecretStore.set()` + reinit IronClaw config |
| `openclaw_remove_key` | `SecretStore.remove()` + reinit IronClaw config |
| `openclaw_get_grants` | Read from `UserConfig` identity grants |
| `openclaw_init` | `init_ironclaw()` (already done in setup) |
| `openclaw_reinit` | `shutdown()` + `init_ironclaw()` (hot reload) |

---

### Step 4.8 — Stub deferred commands

**Side:** Scrappy
**Depends on:** Nothing
**Effort:** 30 min

Commands that have no IronClaw equivalent — return a clear error:

```rust
#[tauri::command]
pub async fn openclaw_fleet_status() -> Result<serde_json::Value, String> {
    Err("Fleet management is not available in embedded mode".into())
}
```

| Command | Status |
|---|---|
| `openclaw_fleet_*` | Stub → "Not available" |
| `openclaw_canvas_*` | Stub → "Coming soon" |
| `openclaw_agents_*` | Stub → "Coming soon" |
| `openclaw_web_login_*` | Stub → "Not applicable in desktop" |

---

### ⬤ Phase 4 Gate

**Milestone:** All 50+ `openclaw_*` commands compile and are registered.
Frontend can call every command without runtime panics. Some may return
placeholder data until IronClaw API modules are fully populated.

---

## Phase 5: Cleanup (Day 8)

**Goal:** Remove old WebSocket bridge code and Node.js dependency.

### Step 5.1 — Delete WebSocket files

**Side:** Scrappy
**Depends on:** Phase 4 Gate (all commands migrated)
**Effort:** 30 min

Delete:
- `src-tauri/src/openclaw/ws_client.rs` (~26 KB)
- `src-tauri/src/openclaw/normalizer.rs` (~27 KB) — **Keep regex patterns
  in a small `sanitizer.rs`** for token sanitization
- `src-tauri/src/openclaw/frames.rs` (~4 KB)
- `src-tauri/src/openclaw/ipc.rs` (~14 KB)

---

### Step 5.2 — Remove `OpenClawManager`

**Side:** Scrappy
**Depends on:** Step 5.1
**Effort:** 30 min

- Delete `OpenClawManager` struct and all references
- Remove from Tauri managed state
- Update `lib.rs` command registration

---

### Step 5.3 — Remove Node.js sidecar

**Side:** Scrappy
**Depends on:** Step 5.2
**Effort:** 30 min

- Remove `node` from `tauri.conf.json` external bins
- Remove `openclaw-engine/` directory
- Remove npm scripts from `package.json` (if any)
- Remove sidecar spawning from `sidecar.rs`
- Update `.gitignore`

---

### Step 5.4 — Extract token sanitizer

**Side:** Scrappy
**Depends on:** Step 5.1
**Effort:** 30 min

Extract the 10 compiled regex patterns from old `normalizer.rs` into a
small `src-tauri/src/openclaw/sanitizer.rs`:

```rust
pub fn sanitize_llm_output(raw: &str) -> String {
    // Apply regex patterns to strip ChatML tokens, Jinja leaks, etc.
    // These are tuned for Scrappy's local model zoo
}
```

Apply in the `TauriChannel::send_status` for `StreamChunk` and
`TauriChannel::respond` for final text.

---

### ⬤ Phase 5 Gate

**Milestone:** `cargo build` succeeds without any WebSocket code.
`node` is not in the binary. App binary size decreases by ~120 MB
(Node.js runtime removed).

---

## Phase 6: Verification (Days 9-10)

**Goal:** End-to-end testing of every feature path.

### Step 6.1 — Core chat flow

- [ ] Send message → streaming tokens appear
- [ ] Multi-turn conversation within a session
- [ ] Switch session → history loads correctly
- [ ] Create new session → empty state
- [ ] Delete session → removed from list

### Step 6.2 — Agentic features

- [ ] Tool execution → ToolUpdate events render tool cards
- [ ] Approval dialog → approve → tool executes → response continues
- [ ] Approval dialog → reject → agent acknowledges
- [ ] Abort mid-turn → response stops, RunStatus "aborted" emitted
- [ ] Multi-step agentic loop (tool → LLM → tool → LLM → final)

### Step 6.3 — Error handling

- [ ] LLM unreachable → UiEvent::Error surfaces in UI (not infinite spinner)
- [ ] Invalid session key → Tauri command returns Err
- [ ] Empty message → Tauri command returns Err
- [ ] Safety rejection → error event with code

### Step 6.4 — Memory & workspace

- [ ] Read SOUL.md → content displays
- [ ] Edit SOUL.md → save → re-read confirms
- [ ] Clear memory → files reset
- [ ] Boot sequence sends SYSTEM_BOOT_SEQUENCE correctly

### Step 6.5 — Skills, extensions, routines

- [ ] List skills → populated list
- [ ] Toggle skill → state changes
- [ ] List routines → populated list
- [ ] Trigger routine → runs in background

### Step 6.6 — Lifecycle

- [ ] App launch → IronClaw initializes → Connected event
- [ ] App quit → background tasks stop → clean exit (no zombie processes)
- [ ] Sidecar crash → IronClaw continues (it's in-process)
- [ ] Key rotation → set new key → reinit → next message uses new key

### Step 6.7 — Performance comparison

- [ ] Measure: time from "Send" to first token (target: <500ms, was ~2000ms with WS)
- [ ] Measure: time from "Send" to command return (target: <10ms)
- [ ] Measure: app binary size reduction (target: -100 MB from removing Node.js)
- [ ] Measure: memory usage reduction (target: -200 MB from eliminating Node.js process)

---

## Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| IronClaw `handle_message()` has implicit dependencies that break when called externally | High | Write integration tests in IronClaw for `handle_message_external()` with mock channels |
| Feature gating breaks transitive dependencies | Medium | Run `cargo check` after each `#[cfg]` addition, not at the end |
| `LogBroadcaster` move causes import failures across the codebase | Medium | Do a `grep_search` for all `log_layer::LogBroadcaster` imports before moving |
| Edition 2024 MSRV 1.92 not available on Scrappy's CI | Low | Pin toolchain in `rust-toolchain.toml` at workspace root |
| `active_session` race condition during concurrent turns | Low | Acceptable for single-user desktop; documented with future fix path |
| Some `StatusUpdate` variants not handled by TauriChannel | Low | Final `_ => return Ok(())` catches unknown variants gracefully |

---

## Post-Integration Enhancements (Future)

These are NOT part of the initial 7-10 day effort but should be planned:

| Enhancement | Priority | Description | Status |
|---|---|---|---|
| Session context in `StatusUpdate` | P1 | Add `session_key: Option<String>` to StatusUpdate variants to eliminate `active_session` concurrency issue | |
| Hot config reload | P2 | Change API keys or model without restart. `reinit_ironclaw()` | |
| Canvas support | ~~P2~~ | ~~Implement `UiEvent::CanvasUpdate` when IronClaw adds canvas feature~~ | ✅ Done (`tools/builtin/canvas.rs`) |
| Sub-agent spawning | P3 | When IronClaw adds `agents.list/spawn`, wire to Scrappy commands | |
| Shared database | P3 | Evaluate unifying ironclaw.db + openclaw.db after stabilization | |
| Fleet management | ~~P3~~ | ~~Desktop fleet support if IronClaw adds peer discovery~~ | ✅ Done (`src/tailscale.rs` — Tailscale discovery) |
| Voice wake word | P2 | `src/voice_wake.rs` scaffold done — needs `cpal` audio capture integration | ✅ Scaffold |
| Talk mode (PTT) | P2 | `src/talk_mode.rs` — CLI audio capture + Whisper API done — local `whisper-rs` deferred | ✅ Scaffold |
| Screen/Camera/Location | ~~P2~~ | ~~Device capability tools~~ | ✅ Done (`screen_capture.rs`, `camera_capture.rs`, `location.rs`) |
| Auto-update checker | ~~P2~~ | ~~GitHub Releases API poller~~ | ✅ Done (`src/update_checker.rs`) |
| QR code pairing | ~~P2~~ | ~~Non-Tailscale pairing fallback~~ | ✅ Done (`src/qr_pairing.rs`) |

---

## Quick Reference: File Inventory

### New Files (IronClaw)

| File | Phase | Lines |
|---|---|---|
| `src/api/mod.rs` | 2.1 | ~15 |
| `src/api/error.rs` | 2.2 | ~40 |
| `src/api/chat.rs` | 2.3 | ~120 |
| `src/api/sessions.rs` | 2.4 | ~100 |
| `src/api/memory.rs` | 2.5 | ~60 |
| `src/api/config.rs` | 2.6 | ~30 |
| `src/api/skills.rs` | 2.7 | ~50 |
| `src/api/routines.rs` | 2.8 | ~50 |
| `src/api/system.rs` | 2.9 | ~80 |
| `src/api/extensions.rs` | 2.10 | ~30 |

### New Files (Scrappy)

| File | Phase | Lines |
|---|---|---|
| `src/openclaw/ironclaw_channel.rs` | 3.2 | ~120 |
| `src/openclaw/ironclaw_bridge.rs` | 3.3 | ~100 |
| `src/openclaw/sanitizer.rs` | 5.4 | ~50 |

### Modified Files (IronClaw)

| File | Phase | Changes |
|---|---|---|
| `Cargo.toml` | 1.2 | Feature flags |
| `src/lib.rs` | 1.3, 1.9 | Feature gates + exports |
| `src/channels/channel.rs` | 1.1 | `StatusUpdate::Error` |
| `src/agent/agent_loop.rs` | 1.4-1.7 | Public methods + background tasks |
| `src/app.rs` | 1.8 | `with_secrets_store()` |

### Deleted Files (Scrappy)

| File | Phase | Size |
|---|---|---|
| `src/openclaw/ws_client.rs` | 5.1 | 26 KB |
| `src/openclaw/normalizer.rs` | 5.1 | 27 KB |
| `src/openclaw/frames.rs` | 5.1 | 4 KB |
| `src/openclaw/ipc.rs` | 5.1 | 14 KB |
| `openclaw-engine/` | 5.3 | ~120 MB |
