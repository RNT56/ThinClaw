# Phase 2: IronClaw Wiring — Detailed Implementation Spec

> **Timeline:** Days 2-3
> **Branch:** `feature/ironclaw-integration`
> **Prerequisite:** IronClaw Phase 1 complete (library crate available)
> **Goal:** Add the IronClaw dependency, create the bridge + channel adapter,
> wire into `lib.rs` setup, and get the first smoke test passing:
> *"App launches, `Connected` event fires, the OpenClaw chat tab shows 'Ready'."*

---

## Overview

Phase 2 creates 3 new files and modifies 2 existing files:

| Action | File | LOC (est.) | Purpose |
|---|---|---|---|
| **CREATE** | `backend/src/openclaw/ironclaw_channel.rs` | ~180 | `impl Channel for TauriChannel` |
| **CREATE** | `backend/src/openclaw/ironclaw_bridge.rs` | ~250 | `IronClawState` lifecycle manager |
| **CREATE** | `backend/src/openclaw/ironclaw_types.rs` | ~60 | Conversion helpers StatusUpdate → UiEvent |
| **MODIFY** | `backend/src/openclaw/mod.rs` | +4 lines | Wire new modules |
| **MODIFY** | `backend/Cargo.toml` | +3 lines | Add ironclaw dependency |

---

## Task 2.1: Add IronClaw Dependency

### `backend/Cargo.toml` — Add to `[dependencies]`

```toml
# IronClaw agent engine (local path during development)
ironclaw = { path = "../../ironclaw/ironclaw", features = ["libsql"] }
```

> **Note:** `features = ["libsql"]` because Scrappy uses libSQL (not Postgres).
> The path is relative: `backend/` → `../../ironclaw/ironclaw/`.

Verify the path resolves correctly:
```bash
ls -la "$(pwd)/../../ironclaw/ironclaw/Cargo.toml"
```

---

## Task 2.2: Create `ironclaw_types.rs`

### Purpose

Conversion functions that map IronClaw's `StatusUpdate` enum (11 variants)
to Scrappy's `UiEvent` enum (13 variants). This is a one-way mapping since
IronClaw emits `StatusUpdate`s and the frontend consumes `UiEvent`s.

### File: `backend/src/openclaw/ironclaw_types.rs`

```rust
//! Conversion layer: IronClaw StatusUpdate → Scrappy UiEvent
//!
//! IronClaw's Channel trait receives StatusUpdate variants during a turn.
//! This module converts them to UiEvent variants that the frontend consumes.

use ironclaw::channels::StatusUpdate;
use serde_json::Value;

use super::sanitizer::strip_llm_tokens;
use super::ui_types::UiEvent;

/// Convert an IronClaw `StatusUpdate` to a Scrappy `UiEvent`.
///
/// The `session_key` and `run_id` are injected from the channel's routing
/// metadata (extracted from IncomingMessage::metadata).
pub fn status_to_ui_event(
    status: StatusUpdate,
    session_key: &str,
    run_id: Option<&str>,
    message_id: &str,
) -> Option<UiEvent> {
    let session_key = session_key.to_string();
    let run_id = run_id.map(|s| s.to_string());

    match status {
        StatusUpdate::Thinking(text) => Some(UiEvent::RunStatus {
            session_key,
            run_id,
            status: "in_flight".into(),
            error: None,
        }),

        StatusUpdate::StreamChunk(delta) => Some(UiEvent::AssistantDelta {
            session_key,
            run_id,
            message_id: message_id.to_string(),
            delta: strip_llm_tokens(&delta),
        }),

        StatusUpdate::ToolStarted { name } => Some(UiEvent::ToolUpdate {
            session_key,
            run_id,
            tool_name: name,
            status: "started".into(),
            input: Value::Null,
            output: Value::Null,
        }),

        StatusUpdate::ToolCompleted { name, success } => Some(UiEvent::ToolUpdate {
            session_key,
            run_id,
            tool_name: name,
            status: if success { "ok" } else { "error" }.into(),
            input: Value::Null,
            output: Value::Null,
        }),

        StatusUpdate::ToolResult { name, preview } => Some(UiEvent::ToolUpdate {
            session_key,
            run_id,
            tool_name: name,
            status: "stream".into(),
            input: Value::Null,
            output: Value::String(preview),
        }),

        StatusUpdate::Status(text) => Some(UiEvent::RunStatus {
            session_key,
            run_id,
            status: text,
            error: None,
        }),

        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
        } => Some(UiEvent::ApprovalRequested {
            approval_id: request_id,
            session_key,
            tool_name,
            input: parameters,
        }),

        StatusUpdate::AuthRequired {
            extension_name,
            auth_url,
            ..
        } => Some(UiEvent::WebLogin {
            provider: extension_name,
            qr_code: None,
            status: auth_url.unwrap_or_else(|| "auth_required".into()),
        }),

        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
        } => Some(UiEvent::WebLogin {
            provider: extension_name,
            qr_code: None,
            status: if success {
                "authenticated".into()
            } else {
                format!("failed: {}", message)
            },
        }),

        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => Some(UiEvent::CanvasUpdate {
            session_key,
            run_id,
            content: serde_json::json!({
                "job_id": job_id,
                "title": title,
                "browse_url": browse_url,
            })
            .to_string(),
            content_type: "json".into(),
            url: Some(browse_url),
        }),

        StatusUpdate::Error { message, code } => Some(UiEvent::Error {
            code: code.unwrap_or_else(|| "turn_failed".into()),
            message,
            details: Value::Null,
        }),
    }
}
```

---

## Task 2.3: Create `ironclaw_channel.rs`

### Purpose

Implements IronClaw's `Channel` trait for Tauri. The key insight:
- `start()` returns a `MessageStream` — we use an `mpsc::channel` and hold the sender
- `respond()` emits `UiEvent::AssistantFinal` via Tauri's event system
- `send_status()` converts `StatusUpdate` → `UiEvent` and emits

### File: `backend/src/openclaw/ironclaw_channel.rs`

```rust
//! Tauri channel adapter for IronClaw.
//!
//! Implements `ironclaw::channels::Channel` to bridge the IronClaw agent
//! engine with Tauri's event system. StatusUpdate emissions are converted
//! to UiEvent and emitted via `AppHandle::emit`.

use std::sync::Arc;

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Wry};
use tokio::sync::{Mutex, mpsc};

use ironclaw::channels::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use ironclaw::error::ChannelError;

use super::ironclaw_types::status_to_ui_event;
use super::sanitizer::strip_llm_tokens;
use super::ui_types::UiEvent;

/// Channel name used for routing. Must match what `api::chat` hardcodes.
const CHANNEL_NAME: &str = "tauri";

/// Event name emitted to the frontend (matches existing `listen("openclaw-event")`)
const EMIT_EVENT: &str = "openclaw-event";

/// Tauri-native channel implementation for IronClaw.
///
/// The channel holds an `mpsc::Sender` that the bridge uses to inject
/// messages from Tauri commands into the agent's message stream.
pub struct TauriChannel {
    app_handle: AppHandle<Wry>,
    /// Sender for injecting messages into the agent loop.
    /// Used by IronClawBridge when Tauri commands produce IncomingMessages.
    inject_tx: mpsc::Sender<IncomingMessage>,
    /// Receiver — taken once in `start()` and converted to a MessageStream.
    inject_rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
}

impl TauriChannel {
    /// Create a new TauriChannel.
    ///
    /// Returns `(channel, sender)` — the sender is stored in `IronClawState`
    /// for Tauri commands to inject messages.
    pub fn new(app_handle: AppHandle<Wry>) -> (Self, mpsc::Sender<IncomingMessage>) {
        let (tx, rx) = mpsc::channel(64);
        let channel = Self {
            app_handle,
            inject_tx: tx.clone(),
            inject_rx: Mutex::new(Some(rx)),
        };
        (channel, tx)
    }

    /// Emit a UiEvent to the frontend.
    fn emit_ui_event(&self, event: &UiEvent) {
        if let Err(e) = self.app_handle.emit(EMIT_EVENT, event) {
            tracing::warn!("Failed to emit UI event: {}", e);
        }
    }
}

#[async_trait]
impl Channel for TauriChannel {
    fn name(&self) -> &str {
        CHANNEL_NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .inject_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: CHANNEL_NAME.into(),
                reason: "start() already called (receiver consumed)".into(),
            })?;

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Extract session_key from the message's metadata or thread_id
        let session_key = msg
            .thread_id
            .as_deref()
            .unwrap_or("default");

        let run_id = msg
            .metadata
            .get("run_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // `respond` is called with the final assistant text
        let event = UiEvent::AssistantFinal {
            session_key: session_key.to_string(),
            run_id,
            message_id: msg.id.to_string(),
            text: strip_llm_tokens(&response.content),
            usage: None, // IronClaw doesn't pass usage through OutgoingResponse
        };
        self.emit_ui_event(&event);

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Extract routing info from metadata
        let session_key = metadata
            .get("thread_id")
            .or_else(|| metadata.get("session_key"))
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let run_id = metadata
            .get("run_id")
            .and_then(|v| v.as_str());

        let message_id = metadata
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if let Some(event) = status_to_ui_event(status, session_key, run_id, message_id) {
            self.emit_ui_event(&event);
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Broadcast as a plain text event — used for heartbeat/self-repair notifications
        let event = UiEvent::AssistantFinal {
            session_key: "system".into(),
            run_id: None,
            message_id: uuid::Uuid::new_v4().to_string(),
            text: strip_llm_tokens(&response.content),
            usage: None,
        };
        self.emit_ui_event(&event);
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        // Tauri is always "healthy" as long as the app is running
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        tracing::info!("TauriChannel shutting down");
        Ok(())
    }
}
```

---

## Task 2.4: Create `ironclaw_bridge.rs`

### Purpose

The lifecycle manager for IronClaw inside Tauri. Handles:
1. **Initialization** — `AppBuilder::new().build_all()` → `Agent::new()`
2. **State management** — Holds `Arc<Agent>`, `BackgroundTasksHandle`
3. **Shutdown** — `agent.shutdown_background(handle)` on app quit

### File: `backend/src/openclaw/ironclaw_bridge.rs`

```rust
//! IronClaw lifecycle bridge for Tauri.
//!
//! Creates, configures, and manages the IronClaw agent engine within
//! the Tauri application. This replaces the Node.js OpenClawEngine
//! gateway process.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

use ironclaw::agent::{Agent, AgentDeps, BackgroundTasksHandle};
use ironclaw::app::{AppBuilder, AppBuilderFlags};
use ironclaw::channels::ChannelManager;
use ironclaw::config::Config as IronClawConfig;
use ironclaw::channels::web::log_layer::LogBroadcaster;
use ironclaw::llm::SessionManager;

use super::ironclaw_channel::TauriChannel;
use super::ui_types::UiEvent;

/// Managed state: holds the running IronClaw agent and background task handle.
///
/// Stored as `tauri::State<IronClawState>` — all Tauri commands access the
/// agent through this. The `Mutex<Option<BackgroundTasksHandle>>` pattern
/// allows `shutdown()` to take ownership via `.take()`.
pub struct IronClawState {
    /// The running agent instance.
    pub agent: Arc<Agent>,
    /// Handle to background tasks (self-repair, heartbeat, routines).
    /// Wrapped in `Mutex<Option<>>` so shutdown can `.take()` it.
    pub bg_handle: Mutex<Option<BackgroundTasksHandle>>,
    /// Sender for injecting messages into the agent's message stream.
    /// Used by Tauri commands to send user messages.
    pub inject_tx: mpsc::Sender<ironclaw::channels::IncomingMessage>,
}

impl IronClawState {
    /// Initialize the IronClaw engine and return managed state.
    ///
    /// This is called once during Tauri's `setup()` hook. It:
    /// 1. Loads IronClaw config from the Scrappy state directory
    /// 2. Creates TauriChannel and registers it
    /// 3. Runs AppBuilder::build_all() (DB, secrets, LLM, tools, extensions)
    /// 4. Creates the Agent
    /// 5. Starts background tasks
    /// 6. Emits a `Connected` event to the frontend
    pub async fn initialize(
        app_handle: tauri::AppHandle<tauri::Wry>,
        state_dir: std::path::PathBuf,
        secrets_store: Option<Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<Self, anyhow::Error> {
        // ── 1. Load config ──────────────────────────────────────────────
        let toml_path = state_dir.join("ironclaw.toml");
        let toml_path_opt = if toml_path.exists() {
            Some(toml_path.clone())
        } else {
            None
        };

        let config = IronClawConfig::from_env()
            .unwrap_or_else(|_| IronClawConfig::default());

        // ── 2. Create TauriChannel ──────────────────────────────────────
        let (tauri_channel, inject_tx) = TauriChannel::new(app_handle.clone());

        // ── 3. Build engine components ──────────────────────────────────
        let session = Arc::new(SessionManager::new());
        let log_broadcaster = Arc::new(LogBroadcaster::new(100));

        let mut builder = AppBuilder::new(
            config,
            AppBuilderFlags::default(),
            toml_path_opt,
            session.clone(),
            log_broadcaster,
        );

        if let Some(store) = secrets_store {
            builder = builder.with_secrets_store(store);
        }

        let components = builder.build_all().await?;

        // ── 4. Create channel manager and register TauriChannel ─────────
        let channel_manager = Arc::new(ChannelManager::new());
        channel_manager.add(Box::new(tauri_channel)).await;

        // ── 5. Create agent ─────────────────────────────────────────────
        let agent_deps = AgentDeps {
            store: components.db.clone(),
            llm: components.llm.clone(),
            cheap_llm: components.cheap_llm.clone(),
            safety: components.safety.clone(),
            tools: components.tools.clone(),
            workspace: components.workspace.clone(),
            extension_manager: components.extension_manager.clone(),
            skill_registry: components.skill_registry.clone(),
            skill_catalog: components.skill_catalog.clone(),
            skills_config: components.config.skills.clone(),
            hooks: components.hooks.clone(),
            cost_guard: components.cost_guard.clone(),
        };

        let agent = Arc::new(Agent::new(
            components.config.agent.clone(),
            agent_deps,
            channel_manager,
            components.config.heartbeat.clone(),
            components.config.hygiene.clone(),
            components.config.routines.clone(),
            Some(components.context_manager.clone()),
            Some(components.session.clone()),
        ));

        // ── 6. Start background tasks ───────────────────────────────────
        let bg_handle = agent.start_background_tasks().await;

        // ── 7. Emit Connected event ─────────────────────────────────────
        use tauri::Emitter;
        let connected = UiEvent::Connected { protocol: 2 };
        if let Err(e) = app_handle.emit("openclaw-event", &connected) {
            tracing::warn!("Failed to emit Connected event: {}", e);
        }

        tracing::info!("IronClaw engine initialized successfully");

        Ok(Self {
            agent,
            bg_handle: Mutex::new(Some(bg_handle)),
            inject_tx,
        })
    }

    /// Gracefully shut down the IronClaw engine.
    ///
    /// Called from Tauri's `on_exit` or `RunEvent::Exit` handler.
    pub async fn shutdown(&self) {
        if let Some(handle) = self.bg_handle.lock().await.take() {
            tracing::info!("Shutting down IronClaw background tasks...");
            self.agent.shutdown_background(handle).await;
        }
        if let Err(e) = self.agent.channels().shutdown_all().await {
            tracing::warn!("Error shutting down channels: {}", e);
        }
        tracing::info!("IronClaw engine shut down");
    }
}
```

---

## Task 2.5: Update `openclaw/mod.rs`

### Current (after Phase 1)

```rust
pub mod sanitizer;
pub mod ui_types;
```

### Add

```rust
pub mod ironclaw_bridge;
pub mod ironclaw_channel;
pub mod ironclaw_types;
pub mod sanitizer;
pub mod ui_types;
```

---

## Task 2.6: Wire into `lib.rs` Setup

### Current Setup Flow

In `lib.rs`, the `run()` function:
1. Creates `AppState`
2. Starts `OpenClawManager` (Node.js gateway)
3. Registers 82+ Tauri commands
4. Sets up the window

### Phase 2 Addition

Add IronClaw initialization **alongside** the existing OpenClawManager setup
(not replacing it yet — that's Phase 3-4). This lets us verify IronClaw
boots without breaking the existing chat flow.

```rust
// In the Tauri setup() closure, after existing state init:

// Initialize IronClaw engine
let app_handle = app.handle().clone();
let state_dir = /* same state_dir used by OpenClawManager */;

// Adapt Scrappy's SecretStore to IronClaw's SecretsStore trait
let secrets_store: Option<Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>> = {
    // TODO Phase 3: implement SecretsStore adapter for Scrappy's keychain
    None
};

tauri::async_runtime::spawn(async move {
    match IronClawState::initialize(app_handle.clone(), state_dir, secrets_store).await {
        Ok(state) => {
            app_handle.manage(state);
            tracing::info!("IronClaw state registered with Tauri");
        }
        Err(e) => {
            tracing::error!("Failed to initialize IronClaw: {}", e);
            // Don't crash — fall back to legacy OpenClawManager
        }
    }
});
```

### Shutdown Hook

In the `RunEvent::Exit` handler:

```rust
RunEvent::Exit => {
    // Existing shutdown code...

    // Shutdown IronClaw
    if let Some(state) = app_handle.try_state::<IronClawState>() {
        tauri::async_runtime::block_on(state.shutdown());
    }
}
```

---

## Task 2.7: Verification

### Build

```bash
cargo build 2>&1 | tail -20
```

Expected: Compiles with `ironclaw` crate. May have dead code warnings
on `ironclaw_bridge` — that's fine (it's not wired to commands yet).

### Smoke Test

1. Run the app: `cargo tauri dev`
2. Open DevTools → Console
3. Look for:
   - `[INFO] IronClaw engine initialized successfully`
   - An `openclaw-event` emission with `{ "kind": "Connected", "protocol": 2 }`
4. The existing chat should still work via legacy OpenClawManager

### Unit Tests

No new tests in Phase 2 — the channel and bridge are integration-tested
by the smoke test. Phase 3 adds command-level tests.

### Git Commit

```bash
git add backend/Cargo.toml \
        backend/src/openclaw/ironclaw_bridge.rs \
        backend/src/openclaw/ironclaw_channel.rs \
        backend/src/openclaw/ironclaw_types.rs \
        backend/src/openclaw/mod.rs \
        backend/src/lib.rs

git commit -m "feat: add IronClaw engine integration (Phase 2)

- ironclaw_channel.rs: impl Channel for TauriChannel (emit UiEvents)
- ironclaw_bridge.rs: IronClawState lifecycle (init + shutdown)
- ironclaw_types.rs: StatusUpdate → UiEvent conversion
- IronClaw boots alongside legacy OpenClawManager (no command rewrite yet)
- Connected event emitted on successful init"
```

---

## Architecture Diagram After Phase 2

```
┌─────────────────────────────────────────────────────────────┐
│                         Tauri App                           │
│                                                             │
│  ┌──────────────┐     ┌──────────────────────────────────┐  │
│  │  lib.rs       │     │  IronClawState (managed state)   │  │
│  │  setup()     │────▶│  ├─ agent: Arc<Agent>             │  │
│  └──────────────┘     │  ├─ bg_handle: Mutex<Option<..>> │  │
│                       │  └─ inject_tx: mpsc::Sender       │  │
│                       └──────────┬───────────────────────┘  │
│                                  │                          │
│                       ┌──────────▼──────────┐               │
│                       │   TauriChannel       │               │
│                       │   impl Channel       │               │
│                       │   ├─ start() → stream│               │
│                       │   ├─ respond() → emit│               │
│                       │   └─ send_status()   │               │
│                       │       → emit UiEvent │               │
│                       └──────────┬───────────┘               │
│                                  │                          │
│                       ┌──────────▼──────────┐               │
│                       │  AppHandle::emit()   │               │
│                       │  "openclaw-event"    │               │
│                       └──────────┬───────────┘               │
│                                  │                          │
│  ┌───────────────────────────────▼──────────────────────┐   │
│  │                    Frontend (React)                    │   │
│  │  listen("openclaw-event") → OpenClawChatView.tsx      │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  Legacy: OpenClawManager (Node.js) — still running    │   │
│  │  Will be removed in Phase 4                           │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

---

## Open Questions for Phase 2

| # | Question | Decision |
|---|---|---|
| 1 | **Config loading**: Should IronClaw load its own `ironclaw.toml` or inherit from Scrappy's `AppState`? | Load separately via `Config::from_env()` + optional toml. Scrappy's config stays for Scrappy concerns (UI prefs, inference router). IronClaw's config is for agent concerns (LLM chain, safety, tools). |
| 2 | **SecretsStore adapter**: Scrappy's keychain → IronClaw's `SecretsStore` trait? | Deferred to Phase 3. Pass `None` for now — IronClaw falls back to env-var key loading. |
| 3 | **Parallel boot**: Should IronClaw init block Tauri startup? | **No.** Spawn async. If init fails, log error and fall back to legacy flow. |
| 4 | **lib.rs wiring complexity**: How much of `lib.rs` needs changing? | Minimal — just 2 insertion points: `setup()` closure and `RunEvent::Exit` handler. |
