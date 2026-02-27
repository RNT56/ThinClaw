# Scrappy-Side IronClaw Integration Roadmap

> **Prerequisite:** IronClaw Phase 1 (library prep) must be complete before
> Scrappy Phase 2 begins. Scrappy Phase 1 can run in parallel.

---

## Phase 1: Preparation (Day 1) — No IronClaw Dependency Needed

### Step 1.1: Extract Token Sanitizer from `normalizer.rs`

**Why first:** The normalizer is being deleted, but the LLM token sanitizer
must survive. Extracting it now means it's safe before any destructive changes.

**File:** `backend/src/openclaw/normalizer.rs` (838 LOC)

**Action:** Create a new small utility file with just the sanitizer:

```
CREATE: backend/src/openclaw/sanitizer.rs (~60 lines)
```

Extract from `normalizer.rs`:
- `static LLM_TOKEN_PATTERNS: LazyLock<Vec<Regex>>` (lines 20-40)
- `pub fn strip_llm_tokens(text: &str) -> String` (lines 42-54)
- The `regex` and `std::sync::LazyLock` imports

This file is self-contained — no dependencies on WsFrame, normalizer logic, etc.

**Verification:** `cargo build` still passes. Add a unit test calling
`strip_llm_tokens("test <|im_start|>assistant hello")` and verifying output.

---

### Step 1.2: Extract UiEvent Types from `normalizer.rs`

**Why:** `UiEvent`, `UiSession`, `UiMessage`, `UiUsage` are used by the
frontend contract and by `TauriChannel`. They must survive normalizer deletion.

**File:** `backend/src/openclaw/normalizer.rs`

**Action:** Create a dedicated types file:

```
CREATE: backend/src/openclaw/ui_types.rs (~120 lines)
```

Move from `normalizer.rs`:
- `pub enum UiEvent` (lines 58-161) — all variants
- `pub struct UiSession` (lines 166-171)
- `pub struct UiMessage` (lines 175-181)
- `pub struct UiUsage` (lines 185-189)
- The `serde` and `serde_json::Value` imports
- All `derive(Serialize, Deserialize, specta::Type, Clone, Debug)` attributes

**Update imports in:**
- `backend/src/openclaw/mod.rs` — change `pub use normalizer::UiEvent` → `pub use ui_types::UiEvent`
- All files that currently `use crate::openclaw::normalizer::UiEvent`

**Verification:** `cargo build` passes. Grep for `normalizer::UiEvent` — zero matches.

---

### Step 1.3: Identify All Command Type Dependencies

**Why:** Before rewriting commands, catalog every type used in commands so we
know what Scrappy-side DTOs to create.

**File:** `backend/src/openclaw/commands/types.rs` (153 LOC)

**Action:** Audit this file. It already contains:
- `OpenClawRpcResponse` — the universal response wrapper
- Various input/output types

**Decision matrix per type:**

| Current Type | After Integration | Action |
|---|---|---|
| `OpenClawRpcResponse` | Keep or simplify | May switch to returning `serde_json::Value` directly |
| `OpenClawStatus` | Keep, populate from IronClaw's `EngineStatus` | Add mapping function |
| Session/history types | Wrap IronClaw types in specta DTOs | Create in `types.rs` |

No code changes yet — just catalog what exists. Document findings in comments.

---

### Step 1.4: Create Cargo Workspace

**Files:**
```
MODIFY: /Cargo.toml (root) — create workspace
MODIFY: backend/Cargo.toml — add ironclaw dependency (commented out for now)
```

**Root `Cargo.toml`:**
```toml
[workspace]
members = ["backend", "ironclaw"]
resolver = "2"
```

**`backend/Cargo.toml` addition (commented, ready for Phase 2):**
```toml
# Uncomment when IronClaw Phase 1 is complete:
# ironclaw = { path = "../ironclaw", default-features = false, features = ["desktop"] }
```

**Verification:** `cargo build -p backend` still works.

---

## Phase 2: Integration Scaffold (Days 2-3) — Requires IronClaw Phase 1

### Step 2.1: Add IronClaw Dependency

**File:** `backend/Cargo.toml`

**Action:** Uncomment the ironclaw dependency:
```toml
[dependencies]
ironclaw = { path = "../ironclaw", default-features = false, features = ["desktop"] }
```

Also add:
```toml
tokio-stream = "0.1"  # For ReceiverStream in TauriChannel
```

**Verification:** `cargo check -p backend` resolves the dependency. Fix any
version conflicts (tokio, serde, uuid, anyhow).

> [!WARNING]
> IronClaw uses Rust edition 2024 (MSRV 1.92). Ensure your toolchain is
> `rustup update && rustup default stable` with >= 1.92.

---

### Step 2.2: Create `ironclaw_channel.rs`

**File:**
```
CREATE: backend/src/ironclaw_channel.rs (~120 lines)
```

**Content:** Copy the `TauriChannel` implementation from spec §3.2 verbatim.

Key points:
- `active_session: Arc<RwLock<String>>` for session key tracking
- Implements all 6 `Channel` trait methods: `name`, `start`, `respond`, `send_status`, `broadcast`, `health_check`
- `respond()` uses `msg.thread_id` as authoritative session key
- `send_status()` maps every `StatusUpdate` variant to the correct `UiEvent`
- `broadcast()` maps to `AssistantInternal` (not `AssistantFinal`)
- `StatusUpdate::Error` maps to `UiEvent::Error`

**Import change in file:**
```rust
use crate::openclaw::ui_types::UiEvent;  // Uses the extracted types from Step 1.2
```

**Verification:** `cargo check` — the `Channel` trait impl must compile against
IronClaw's actual trait definition. Fix any method signature mismatches.

---

### Step 2.3: Create `ironclaw_bridge.rs`

**File:**
```
CREATE: backend/src/ironclaw_bridge.rs (~120 lines)
```

**Content:** Copy the bridge from spec §3.1, including:
- `IronClawState` struct with `components`, `agent`, `channel`, `background`
- `init_ironclaw()` async function
- `inject_granted_keys()` helper

**`IronClawState` fields:**
```rust
pub struct IronClawState {
    pub components: Arc<AppComponents>,
    pub agent: Arc<Agent>,
    pub channel: Arc<TauriChannel>,
    pub background: tokio::sync::Mutex<Option<ironclaw::agent::BackgroundTasksHandle>>,
}
```

**`inject_granted_keys()` — map ALL 21 providers:**

Use the grant flags from `UserConfig` / `OpenClawConfig.identity`:
```rust
fn inject_granted_keys(config: &mut Config, store: &SecretStore, uc: &UserConfig) {
    // Primary providers
    if uc.anthropic_granted { config.llm.nearai.api_key = store.get("anthropic"); }
    if uc.openai_granted    { config.llm.nearai.openai_key = store.get("openai"); }
    if uc.groq_granted      { /* ... */ }
    if uc.gemini_granted    { /* ... */ }
    if uc.openrouter_granted { /* ... */ }
    // Implicit providers
    if uc.xai_granted       { /* ... */ }
    if uc.venice_granted    { /* ... */ }
    if uc.together_granted  { /* ... */ }
    // ... etc for all 21
    // Bedrock (3 keys)
    if uc.bedrock_granted {
        config.llm.bedrock.access_key = store.get("bedrock_access_key_id");
        config.llm.bedrock.secret_key = store.get("bedrock_secret_access_key");
        config.llm.bedrock.region = store.get("bedrock_region");
    }
}
```

> [!IMPORTANT]
> The exact config field paths (`config.llm.nearai.*`, `config.llm.bedrock.*`)
> depend on IronClaw's actual `Config` struct layout. Use IronClaw's docs or
> source to find the correct paths.

**Verification:** `cargo check` — struct/function signatures compile.

---

### Step 2.4: Register IronClaw State in `lib.rs`

**File:** `backend/src/lib.rs`

**Changes:**

1. **Add module declarations** (after existing `mod` block, ~line 20):
```rust
mod ironclaw_bridge;
mod ironclaw_channel;
```

2. **Add IronClaw init in setup** (replace lines 401-427):

Replace the entire `openclaw_state.init_config()` + auto-start gateway block with:
```rust
// ── IronClaw Engine Init ────────────────────────────────────────
let secret_store_ref = handle.state::<crate::secret_store::SecretStore>();
let user_config_ref = handle.state::<crate::config::UserConfig>();

match crate::ironclaw_bridge::init_ironclaw(&handle, &secret_store_ref, &user_config_ref).await {
    Ok(ironclaw_state) => {
        handle.manage(ironclaw_state);
        use tauri::Emitter;
        handle.emit("openclaw-event", crate::openclaw::UiEvent::Connected { protocol: 1 }).ok();
        println!("[main] IronClaw engine initialized.");
    }
    Err(e) => {
        eprintln!("[main] Failed to init IronClaw: {}", e);
        use tauri::Emitter;
        handle.emit("openclaw-event", crate::openclaw::UiEvent::Error {
            code: "init_failed".into(),
            message: e.to_string(),
            details: serde_json::Value::Null,
        }).ok();
    }
}
```

3. **Remove old init** (lines 401-427):
   - Delete `let openclaw_state = handle.state::<openclaw::OpenClawManager>();`
   - Delete `openclaw_state.init_config()` call
   - Delete the entire auto-start gateway block

4. **Keep OpenClawManager registration** temporarily — some commands still need
   it until they're rewritten in Phase 3. Remove in Phase 4.

5. **Add shutdown handler** (in `on_window_event` or `on_event`):
```rust
app.on_window_event(|window, event| {
    if let tauri::WindowEvent::CloseRequested { .. } = event {
        let handle = window.app_handle();
        if let Some(state) = handle.try_state::<ironclaw_bridge::IronClawState>() {
            tauri::async_runtime::block_on(async {
                if let Some(bg) = state.background.lock().await.take() {
                    state.agent.shutdown(bg).await;
                }
            });
        }
    }
});
```

**Verification:** App launches. Console shows `[main] IronClaw engine initialized.`
Frontend receives `Connected { protocol: 1 }` event.

---

### Step 2.5: First Smoke Test

At this point the app should:
- ✅ Launch without crashing
- ✅ Emit `Connected` event to frontend
- ✅ Still have all old commands registered (they don't work but don't crash)
- ✅ Show IronClaw initialized in console logs

The OpenClaw tab won't work yet — commands still try to use the old WS bridge.
That's expected and fixed in Phase 3.

---

## Phase 3: Command Migration (Days 3-6) — The Tedious Part

### Command Migration Strategy

All 82 registered `openclaw_*` commands need rewriting. They fall into groups:

**Group A: Chat/Session (12 commands) — Highest priority, test first**
**Group B: Memory/Workspace (6 commands)**
**Group C: Config/Keys (30+ commands) — Most numerous but simplest**
**Group D: Skills/Cron (8 commands)**
**Group E: System/Gateway (8 commands)**
**Group F: Fleet/Deploy/Web Login (6 commands) — Stub or defer**
**Group G: Canvas/Orchestration (4 commands) — Stub**

### Step 3.1: Rewrite `sessions.rs` — Group A (765 LOC → ~200 LOC)

**File:** `backend/src/openclaw/commands/sessions.rs`

**Current pattern (every command):**
```rust
pub async fn openclaw_send_message(
    state: State<'_, OpenClawManager>,  // OLD
    ...
) -> Result<OpenClawRpcResponse, String> {
    let res = ws_rpc(state, |h| h.chat_send(...)).await?;  // OLD: WS round-trip
    Ok(OpenClawRpcResponse { success: true, data: Some(res), error: None })
}
```

**New pattern:**
```rust
pub async fn openclaw_send_message(
    state: State<'_, IronClawState>,  // NEW
    session_key: String, text: String, deliver: bool,
) -> Result<serde_json::Value, String> {
    // Set active session BEFORE spawning
    *state.channel.active_session.write().await = session_key.clone();

    let result = ironclaw::api::chat::send_message(
        &state.agent, &session_key, &text, deliver,
    ).await.map_err(|e| e.to_string())?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}
```

**All 12 commands to rewrite:**

| Command | Old RPC | New API Call | Pattern |
|---|---|---|---|
| `openclaw_send_message` | `h.chat_send()` | `api::chat::send_message()` | 🔥 Spawn |
| `openclaw_abort_chat` | `h.chat_abort()` | `api::chat::abort()` | ⚡ Control |
| `openclaw_resolve_approval` | `h.exec_approval()` | `api::chat::resolve_approval()` | 🔥 Spawn |
| `openclaw_get_sessions` | `h.sessions_list()` | `api::sessions::list_sessions()` | 📖 Query |
| `openclaw_get_history` | `h.chat_history()` | `api::sessions::get_history()` | 📖 Query |
| `openclaw_delete_session` | `h.sessions_delete()` | `api::sessions::delete_session()` | ✏️ Write |
| `openclaw_reset_session` | `h.sessions_reset()` | `api::sessions::clear_session()` | ✏️ Write |
| `openclaw_subscribe_session` | `h.chat_subscribe()` | No-op (events are global via `emit`) | Return Ok |
| `openclaw_spawn_session` | `h.sessions_spawn()` | `api::sessions::create_session()` | ✏️ Write |
| `openclaw_agents_list` | `h.agents_list()` | Stub: return empty list | 📖 Query |
| `openclaw_canvas_push` | `h.canvas_push()` | Stub: return Ok | Deferred |
| `openclaw_canvas_navigate` | `h.canvas_navigate()` | Stub: return Ok | Deferred |

> [!TIP]
> For `openclaw_send_message` and `openclaw_resolve_approval`, remember to set
> `active_session` on the TauriChannel before calling the API.

**Smoke test after this step:** Send a message from the OpenClaw tab. Verify:
- AssistantDelta tokens appear in real-time
- ToolUpdate cards appear
- AssistantFinal appears with complete response
- Session key matches (events aren't silently dropped)

---

### Step 3.2: Rewrite `rpc.rs` — Groups B, D (566 LOC → ~250 LOC)

**File:** `backend/src/openclaw/commands/rpc.rs`

**Memory/Workspace commands (Group B):**

| Command | New API Call |
|---|---|
| `openclaw_get_memory` | `api::memory::get_file(ws, "MEMORY.md")` |
| `openclaw_save_memory` | `api::memory::write_file(ws, "MEMORY.md", content)` |
| `openclaw_clear_memory` | `api::memory::clear(ws, target)` |
| `openclaw_get_file` | `api::memory::get_file(ws, path)` |
| `openclaw_write_file` | `api::memory::write_file(ws, path, content)` |
| `openclaw_list_workspace_files` | `api::memory::list_files(ws)` |

**Skills commands (Group D1):**

| Command | New API Call |
|---|---|
| `openclaw_skills_list` | `api::skills::list_skills()` |
| `openclaw_skills_status` | `api::skills::get_status()` |
| `openclaw_skills_toggle` | `api::skills::toggle_skill()` |
| `openclaw_install_skill_repo` | `api::skills::install_skill()` |
| `openclaw_install_skill_deps` | `api::skills::install_deps()` |

**Cron commands (Group D2):**

| Command | New API Call |
|---|---|
| `openclaw_cron_list` | `api::routines::list_routines()` |
| `openclaw_cron_run` | `api::routines::trigger_routine()` |
| `openclaw_cron_history` | `api::routines::get_history()` |

**Config commands:**

| Command | New API Call |
|---|---|
| `openclaw_config_schema` | `api::config::get_schema()` |
| `openclaw_config_get` | `api::config::get_config()` |
| `openclaw_config_set` | `api::config::set_config()` |
| `openclaw_config_patch` | `api::config::patch_config()` |

**Remaining RPC commands:**

| Command | New API Call |
|---|---|
| `openclaw_system_presence` | Stub: return single-node |
| `openclaw_logs_tail` | `api::system::tail_logs()` |
| `openclaw_update_run` | Stub: return not-applicable |
| `openclaw_web_login_whatsapp` | Stub: return not-supported |
| `openclaw_web_login_telegram` | Stub: return not-supported |

---

### Step 3.3: Rewrite `gateway.rs` — Group E (535 LOC → ~80 LOC)

**File:** `backend/src/openclaw/commands/gateway.rs`

This file currently handles Node.js process lifecycle. After IronClaw, it
becomes dramatically simpler:

| Command | Before | After |
|---|---|---|
| `openclaw_start_gateway` | Spawns Node.js process, connects WS | No-op (IronClaw auto-starts at init) or re-init |
| `openclaw_stop_gateway` | Kills Node.js process, closes WS | `agent.shutdown(handle)` |
| `openclaw_get_status` | Aggregates config + process + WS state | Build `OpenClawStatus` from `IronClawState` |
| `openclaw_get_diagnostics` | WS RPC `diagnostics` | `api::system::diagnostics()` |

**`openclaw_get_status` is the most complex rewrite.** It populates a ~40 field
struct. The mapping:

```rust
pub async fn openclaw_get_status(
    ironclaw: State<'_, IronClawState>,
    secret_store: State<'_, SecretStore>,
    config_state: State<'_, OpenClawManager>,  // Keep temporarily for identity
) -> Result<OpenClawStatus, String> {
    let engine = ironclaw::api::system::get_status(&ironclaw.components);
    let identity = config_state.get_config().await;

    Ok(OpenClawStatus {
        gateway_running: true,           // Always true when IronClaw is initialized
        ws_connected: true,              // Always true (in-process)
        setup_completed: engine.setup_completed,
        selected_cloud_brain: identity.selected_cloud_brain,
        selected_cloud_model: identity.selected_cloud_model,
        // ... map all 40 fields from identity + engine status + secret_store
    })
}
```

---

### Step 3.4: Rewrite `keys.rs` — Group C (1029 LOC → ~400 LOC)

**File:** `backend/src/openclaw/commands/keys.rs`

This is the **largest file** by line count. It has ~30 commands for
saving/getting API keys per provider. The pattern is identical for each:

**Current pattern:**
```rust
pub async fn openclaw_save_anthropic_key(
    state: State<'_, OpenClawManager>,
    secret_store: State<'_, SecretStore>,
    key: String,
) -> Result<(), String> {
    secret_store.set("anthropic", &key).map_err(|e| e.to_string())?;
    // Also: write to identity.json, regenerate auth-profiles.json, restart WS
    ...
}
```

**New pattern:**
```rust
pub async fn openclaw_save_anthropic_key(
    ironclaw: State<'_, IronClawState>,
    secret_store: State<'_, SecretStore>,
    key: String,
) -> Result<(), String> {
    secret_store.set("anthropic", &key).map_err(|e| e.to_string())?;
    // Update IronClaw's config if needed (hot-reload key)
    // NOTE: May require IronClaw API for config updates:
    // ironclaw::api::config::update_key(&ironclaw.components, "anthropic", &key).await?;
    Ok(())
}
```

**Key simplification:** Remove all `auth-profiles.json` generation. IronClaw
doesn't need this file — keys are passed as Config values.

**Commands in this group (all same pattern):**
- `save/get_anthropic_key`, `save/get_openai_key`, `save/get_groq_key`
- `save/get_gemini_key`, `save/get_openrouter_key`, `save/get_brave_key`
- `save/get_implicit_provider_key`, `save/get_bedrock_credentials`
- `set_hf_token`, `save_selected_cloud_model`, `select_openclaw_brain`
- `save_cloud_config`, `toggle_secret_access`
- `add/remove/toggle_custom_secret`
- `toggle_node_host`, `toggle_local_inference`, `toggle_expose_inference`
- `toggle_auto_start`, `set_setup_completed`, `set_dev_mode_wizard`
- `save_slack_config`, `save_telegram_config`, `save_gateway_settings`
- `add/remove_agent_profile`, `sync_local_llm`

Many of these write to `identity.json` — that logic stays (Scrappy owns
config persistence). The change is: **remove the WS restart / auth-profiles
regeneration**, and **optionally hot-reload the key into IronClaw**.

---

### Step 3.5: Rewrite `fleet.rs` and `extra_commands.rs`

**`fleet.rs` (396 LOC → ~30 LOC stub):**
```rust
pub async fn openclaw_get_fleet_status(...) -> Result<Value, String> {
    Ok(serde_json::json!({ "nodes": [], "status": "not_available" }))
}
pub async fn openclaw_broadcast_command(...) -> Result<Value, String> {
    Err("Fleet management not available in desktop mode".into())
}
```

**`extra_commands.rs` (79 LOC → ~30 LOC):**
```rust
pub async fn openclaw_switch_to_profile(
    ironclaw: State<'_, IronClawState>,
    // ... profile_id, etc.
) -> Result<(), String> {
    // Write identity.json with new profile, then optionally
    // update IronClaw's system prompt via api::memory::write_file(SOUL.md)
    ...
}
pub async fn openclaw_test_connection(
    ironclaw: State<'_, IronClawState>,
) -> Result<(), String> {
    ironclaw::api::system::health_check().map_err(|e| e.to_string())
}
```

---

### Step 3.6: Update `lib.rs` Command Registration

**File:** `backend/src/lib.rs`

After all commands are rewritten, update the `invoke_handler` to use the new
modules. The command **names stay the same** (TypeScript bindings don't change),
only the implementations change.

No changes to the `invoke_handler!()` macro call itself — the function names
remain identical. The difference is internal (they now take
`State<'_, IronClawState>` instead of `State<'_, OpenClawManager>`).

---

## Phase 4: Cleanup (Day 7) — Delete Dead Code

### Step 4.1: Delete WS Bridge Files

```
DELETE: backend/src/openclaw/ws_client.rs        (748 LOC)
DELETE: backend/src/openclaw/frames.rs           (155 LOC)
DELETE: backend/src/openclaw/ipc.rs              (425 LOC)
DELETE: backend/src/openclaw/normalizer.rs       (838 LOC) — sanitizer + types already extracted
```

### Step 4.2: Update `openclaw/mod.rs`

```rust
// BEFORE:
pub mod commands;
pub mod config;
pub mod deploy;
pub mod extra_commands;
pub mod fleet;
mod frames;
pub mod ipc;
mod normalizer;
mod ws_client;

pub use commands::OpenClawManager;
pub use config::OpenClawConfig;
pub use frames::{WsError, WsFrame};
pub use normalizer::{UiEvent, UiMessage, UiSession, UiUsage};

// AFTER:
pub mod commands;
pub mod config;
pub mod deploy;
pub mod extra_commands;
pub mod fleet;
pub mod sanitizer;    // NEW: extracted from normalizer
pub mod ui_types;     // NEW: extracted from normalizer

pub use config::OpenClawConfig;
pub use ui_types::{UiEvent, UiMessage, UiSession, UiUsage};
// OpenClawManager may still be needed for config — keep until fully replaced
```

### Step 4.3: Remove `OpenClawManager` from `commands/mod.rs`

**File:** `backend/src/openclaw/commands/mod.rs` (349 LOC → ~30 LOC)

Delete:
- `OpenClawEngineProcess` struct (process lifecycle)
- `OpenClawManager` struct (WS handle, process management)
- `ws_rpc()` helper function
- `start_openclaw_engine_process()` (200+ lines of Node.js spawning)
- `stop_openclaw_engine_process()`

Keep:
- Module structure (`mod keys; mod rpc; mod sessions; ...`)
- `pub use` re-exports

### Step 4.4: Remove Node.js Sidecar from `sidecar.rs`

**File:** `backend/src/sidecar.rs`

Remove the `node` sidecar from `SidecarManager`. Keep all inference sidecars:
- `llama-server` ✅ keep
- `whisper-server` ✅ keep
- `embedding-server` ✅ keep
- `node` / `openclaw-engine` ❌ **remove**

### Step 4.5: Update `tauri.conf.json`

Remove Node.js from external bins:
```json
// DELETE:
"externalBin": ["node-aarch64-apple-darwin", ...]
```

Keep all inference engine entries.

### Step 4.6: Delete Node.js Engine Bundle

```
DELETE: openclaw-engine/           (entire directory — npm package)
DELETE: scripts/download_node.js   (if exists)
```

This saves **~120 MB** from the app bundle.

### Step 4.7: Remove Dead Dependencies from `Cargo.toml`

Check if these are still needed after removing WS bridge:
- `ed25519-dalek` — was used for WS auth signatures. **Remove** if no other user.
- `pkcs8` — same. **Remove** if no other user.
- Any WebSocket client libraries (`tungstenite`, etc.)

---

## Phase 5: Verification (Day 8)

### Step 5.1: Full Command Audit

Run every single `openclaw_*` command from the frontend and verify:

**Chat commands:**
- [ ] Send message → tokens stream → tool cards appear → final response
- [ ] Abort mid-stream → stops cleanly
- [ ] Approval flow → tool card → approve/deny → continues/stops
- [ ] Get sessions → list renders
- [ ] Delete session → removed from list
- [ ] Get history → previous messages load
- [ ] Reset session → messages cleared
- [ ] Spawn session → new empty session

**Memory/Workspace:**
- [ ] Get memory (MEMORY.md) → content loads
- [ ] Save memory → persists across restart
- [ ] Get/write workspace files → SOUL.md, BOOTSTRAP.md accessible
- [ ] List workspace files → shows all markdown files
- [ ] Clear memory → factory reset works

**Keys/Config:**
- [ ] Save API key → stored in Keychain + passed to IronClaw
- [ ] Toggle grant → provider enabled/disabled
- [ ] Set cloud model → model selection updates
- [ ] Get status → all 40 fields populated correctly

**Skills/Cron:**
- [ ] List skills → rendered in Settings
- [ ] Toggle skill → enables/disables
- [ ] List cron → shows scheduled tasks
- [ ] Trigger cron → job runs

**Boot sequence:**
- [ ] Close and reopen OpenClaw tab → SYSTEM_BOOT_SEQUENCE sent
- [ ] SOUL.md content appears in boot message
- [ ] MEMORY.md loaded correctly

### Step 5.2: Regression Testing

- [ ] Chat tab (Rig agent) still works — completely independent
- [ ] Settings panel loads without errors
- [ ] Model browser still works
- [ ] Image generation still works
- [ ] TTS/STT still works
- [ ] Spotlight bar still works
- [ ] Global shortcuts still work
- [ ] Tray icon still works

### Step 5.3: Performance Comparison

| Metric | Before (WS Bridge) | After (In-Process) | Expected |
|---|---|---|---|
| Time-to-first-token | ~200ms (WS overhead) | ~50ms | 4x faster |
| App bundle size | ~250MB (Node.js) | ~130MB | 120MB smaller |
| Memory (idle) | ~300MB (Node + WS) | ~150MB | 50% less |
| Startup time | ~3s (Node spawn) | ~0.5s | 6x faster |

---

## File Change Summary

### New Files (3)
| File | LOC | Purpose |
|---|---|---|
| `backend/src/ironclaw_bridge.rs` | ~120 | Init + state wrapper |
| `backend/src/ironclaw_channel.rs` | ~120 | TauriChannel impl |
| `backend/src/openclaw/sanitizer.rs` | ~60 | Token sanitizer (extracted) |
| `backend/src/openclaw/ui_types.rs` | ~120 | UiEvent enum (extracted) |

### Modified Files (8)
| File | Change |
|---|---|
| `backend/Cargo.toml` | Add `ironclaw` dep |
| `backend/src/lib.rs` | IronClaw init, shutdown, mod declarations |
| `backend/src/openclaw/mod.rs` | Remove dead modules, add new ones |
| `backend/src/openclaw/commands/mod.rs` | Remove OpenClawManager, ws_rpc |
| `backend/src/openclaw/commands/sessions.rs` | Rewrite all 12 commands |
| `backend/src/openclaw/commands/rpc.rs` | Rewrite all ~18 commands |
| `backend/src/openclaw/commands/gateway.rs` | Simplify to ~80 LOC |
| `backend/src/openclaw/commands/keys.rs` | Remove auth-profiles, simplify saves |

### Deleted Files (5 + directory)
| File | LOC | Saved |
|---|---|---|
| `backend/src/openclaw/ws_client.rs` | 748 | WS actor |
| `backend/src/openclaw/normalizer.rs` | 838 | Event normalizer (parts extracted) |
| `backend/src/openclaw/frames.rs` | 155 | WS frame types |
| `backend/src/openclaw/ipc.rs` | 425 | Reverse-RPC |
| `openclaw-engine/` | ~120 MB | Entire Node.js bundle |

### Net Change
- **~420 new lines** (bridge, channel, sanitizer, ui_types)
- **~2,166 deleted lines** (WS bridge files)
- **~2,800 rewritten lines** (commands)
- **~120 MB deleted** (Node.js bundle)
