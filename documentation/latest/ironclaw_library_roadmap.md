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
- **File:** `src/channels/channel.rs` · **Effort:** 15 min
- Added `Error { message: String, code: Option<String> }` to `StatusUpdate`

### Step 1.2 — Add feature flags to `Cargo.toml`
- **Effort:** 30 min
- Features: `default`, `desktop` (libsql + html-to-markdown), `web-gateway`, `repl`, `tunnel`, `docker-sandbox`, `full`

### Step 1.3 — Gate heavy modules behind feature flags
- **Effort:** 1-2 hours
- `#[cfg(feature = "...")]` on: `channels_web_server`, `channels_repl`, `tunnel`, `orchestrator`, `worker`
- Moved `LogBroadcaster` to shared location

### Step 1.4 — Make `Agent::handle_message()` public
- **Effort:** 15 min — `pub async fn handle_message_external()`

### Step 1.5 — Add `Agent::inject_context()`
- **Effort:** 20 min — Persists user message without triggering a turn

### Step 1.6 — Add `Agent::cancel_turn()`
- **Effort:** 15 min — Sets interrupt flag on active turn

### Step 1.7 — Extract background tasks from `Agent::run()`
- **Effort:** 1-2 hours
- Created `BackgroundTasksHandle` struct
- Extracted `start_background_tasks()` / `shutdown()`
- Self-repair, session pruning, heartbeat, routine engine all managed

### Step 1.8 — Ensure `AppBuilder` accepts keys via Config
- **Effort:** 30 min — Added `with_secrets_store()` method

### Step 1.9 — Export types from `lib.rs`
- **Effort:** 15 min — `pub mod api`, `pub mod prelude`, public re-exports

---

## Phase 2: IronClaw API Surface (Days 2-3)

**Goal:** Build the `ironclaw::api` module — framework-agnostic functions
that expose all agent capabilities as callable Rust functions.

### API Modules Created

| Module | Key Functions |
|--------|-------------|
| `api/error.rs` | `ApiError` enum (InvalidInput, SessionNotFound, Agent, Serialization, UuidParse) |
| `api/chat.rs` | `send_message()`, `resolve_approval()`, `abort()` |
| `api/sessions.rs` | `list_sessions()`, `get_history()`, `create_session()`, `delete_session()`, `clear_session()` |
| `api/memory.rs` | `get_file()`, `write_file()`, `list_files()`, `clear()`, `search()` |
| `api/config.rs` | `get_config()`, `set_config()` |
| `api/skills.rs` | `list_skills()`, `toggle_skill()`, `install_skill()` |
| `api/routines.rs` | `list_routines()`, `trigger_routine()` |
| `api/system.rs` | `get_status()`, `health_check()`, `list_models()`, `tail_logs()`, `diagnostics()` |
| `api/extensions.rs` | `list_extensions()`, `toggle_extension()` |

---

## Phase 3-4: Scrappy Integration (Days 4-7)

> See `ironclaw_integration_roadmap.md` for the Scrappy-side counterpart.

### Step 3.1 — Add IronClaw as dependency
- `ironclaw = { path = "../../ironclaw/ironclaw", features = ["desktop"] }`

### Step 3.2 — Create `ironclaw_channel.rs`
- `TauriChannel` implements `Channel` trait
- StatusUpdate → UiEvent conversion via `ironclaw_types.rs`

### Step 3.3 — Create `ironclaw_bridge.rs`
- `IronClawState` struct with `RwLock<Option<IronClawInner>>`
- `start()` / `stop()` / `shutdown()` lifecycle

### Step 3.4 — Wire IronClaw into Tauri `setup()`
- Auto-starts on app launch, graceful shutdown on exit

### Step 4.1-4.8 — Command Migration (50+ commands)
- Sessions (12), Chat (5), Memory (8), Skills (6), Cron (4), Config/System (8), Keys (adapted), Stubs (deferred)

---

## Phase 5-6: Cleanup & Verification

### Deleted Files
| File | LOC |
|------|-----|
| `ws_client.rs` | 748 |
| `normalizer.rs` | 838 |
| `frames.rs` | 155 |
| `ipc.rs` | 425 |
| `openclaw-engine/` | ~120 MB |

### Performance Targets

| Metric | Before (WS Bridge) | After (In-Process) | Expected |
|---|---|---|---|
| Time-to-first-token | ~200ms (WS overhead) | ~50ms | 4x faster |
| App bundle size | ~250MB (Node.js) | ~130MB | 120MB smaller |
| Memory (idle) | ~300MB (Node + WS) | ~150MB | 50% less |
| Startup time | ~3s (Node spawn) | ~0.5s | 6x faster |

---

## Post-Integration Enhancements

| Enhancement | Priority | Status |
|---|---|---|
| Session context in `StatusUpdate` | P1 | |
| Hot config reload | P2 | |
| Canvas support | ~~P2~~ | ✅ Done (`tools/builtin/canvas.rs`) |
| Sub-agent spawning | P3 | |
| Shared database | P3 | |
| Fleet management | ~~P3~~ | ✅ Done (`src/tailscale.rs`) |
| Voice wake word | P2 | ✅ Scaffold |
| Talk mode (PTT) | P2 | ✅ Scaffold |
| Screen/Camera/Location | ~~P2~~ | ✅ Done |
| Auto-update checker | ~~P2~~ | ✅ Done (`src/update_checker.rs`) |
| QR code pairing | ~~P2~~ | ✅ Done (`src/qr_pairing.rs`) |

---

## File Inventory

### New Files (IronClaw Library)
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

### New Files (Scrappy Integration)
| File | Phase | Lines |
|---|---|---|
| `ironclaw_bridge.rs` | 3.3 | ~300 |
| `ironclaw_channel.rs` | 3.2 | ~157 |
| `ironclaw_secrets.rs` | 2.5b | ~199 |
| `ironclaw_types.rs` | 2.2 | ~134 |
| `sanitizer.rs` | 1.1 | ~95 |
| `ui_types.rs` | 1.2 | ~130 |

### Modified Files (IronClaw Library)
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
| `ws_client.rs` | 5.1 | 26 KB |
| `normalizer.rs` | 5.1 | 27 KB |
| `frames.rs` | 5.1 | 4 KB |
| `ipc.rs` | 5.1 | 14 KB |
| `openclaw-engine/` | 5.3 | ~120 MB |
