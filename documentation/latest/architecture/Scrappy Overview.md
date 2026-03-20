# Scrappy — Full Application Overview

> **Last updated:** 2026-03-05  
> **Architecture:** Tauri v2 (Rust backend + React/TypeScript frontend)  
> **Platforms:** macOS (primary, Apple Silicon + Intel), Linux (planned)  
> **License:** GPL-3.0-only

---

## 1. What is Scrappy?

Scrappy is an AI-powered macOS desktop application that combines:

1. **A local-first AI assistant** — runs LLMs directly on your Mac via llama.cpp, MLX, Ollama, or vLLM
2. **A multi-channel AI agent** — powered by the IronClaw engine for agentic workflows with tools, memory, and automation
3. **An image generation studio** — local Stable Diffusion / FLUX / MLX + cloud (Imagen 3, DALL-E 3, Stability AI, fal.ai, Together AI)
4. **A cloud-connected brain** — with 14 cloud LLM providers and 7 cloud storage providers (S3-compatible, Google Drive, Dropbox, OneDrive, iCloud, SFTP, WebDAV)

**Strategic positioning:** IronClaw is the *headless agent engine* (backend); Scrappy is the *intelligent operating system* (application) that wraps IronClaw and adds world-class native inference, model management, and a premium UI experience.

---

## 2. Dual-Agent Architecture

Scrappy runs **two independent AI pipelines** side-by-side:

| Dimension | Chat Tab (Rig Agent) | OpenClaw Tab (IronClaw Agent) |
|-----------|---------------------|-------------------------------|
| **Purpose** | Fast, stateless chat + tool use | Complex agentic workflows with memory, tools, routines |
| **Engine** | `rig-core` (Rust library) via `Orchestrator::run_turn()` | IronClaw library crate (in-process, no WS/sidecar) |
| **Streaming** | `tauri::Channel<StreamChunk>` (per-command typed channel) | `TauriChannel` → `app.emit("openclaw-event", UiEvent)` |
| **Tools** | Rhai sandbox + MCP tools + host (web_search, calculator, etc.) | IronClaw `ToolRegistry` + WASM sandbox + safety layer |
| **Memory** | SQLite via `sqlx` (Scrappy-owned) | IronClaw workspace (libSQL) with hybrid BM25+vector search |
| **Context** | Manual prompt assembly in `chat.rs` | Auto context compaction, post-compaction injection, skills routing |
| **State** | Ephemeral (per-session messages in DB) | Persistent sessions, per-sender isolation, global sessions |
| **Database** | `scrappy.db` (SQLite via `sqlx`) | `ironclaw.db` (libSQL – patched to coexist with sqlx) |

### Why Two Engines?

- **Chat Tab** — optimized for quick queries, model testing, and experimentation. No agent overhead. Supports ReAct-style tool orchestration via Rhai scripting.
- **OpenClaw Tab** — full IronClaw agent with multi-turn conversations, workspace memory, tool approval workflows, cron scheduling, and multi-channel broadcasting.

---

## 3. Inference Engines (Compile-Time Feature Selection)

Each build of Scrappy targets **exactly one** local inference engine, selected by Cargo feature flags at compile time. All engines expose an **OpenAI-compatible HTTP API** on a local port, making the rest of the stack engine-agnostic.

### 3.1 Available Engines

```toml
# backend/Cargo.toml
[features]
default = ["llamacpp"]
llamacpp = []     # Bundled llama-server sidecar — all platforms
mlx = []          # External Python process via uv — macOS Apple Silicon only
vllm = []         # External Python process via uv — Linux CUDA only
ollama = []       # Detect existing Ollama daemon — all platforms
```

| Engine | Feature Flag | Platform | Model Format | Bootstrap | Description |
|--------|-------------|----------|-------------|-----------|-------------|
| **llama.cpp** | `llamacpp` (default) | All | GGUF (single file) | None | Bundled `llama-server` sidecar. Metal/CUDA/CPU. Zero setup. |
| **MLX** | `mlx` | macOS Apple Silicon | MLX Safetensors (directory) | Python venv via `uv` (~2-3 min first launch) | Apple's MLX framework. Best performance on M-series. Uses `mlx-openai-server`. |
| **vLLM** | `vllm` | Linux CUDA | AWQ/GPTQ/FP16 (directory) | Python venv via `uv` (~5-10 min first launch) | High-throughput inference for NVIDIA GPUs. OpenAI-compatible API. |
| **Ollama** | `ollama` | All | GGUF (via Ollama registry) | External daemon (user installs from ollama.ai) | Delegates to a running Ollama daemon on localhost:11434. |
| **Cloud Only** | (none) | All | N/A | None | No local inference. Cloud providers only. |

### 3.2 Engine Trait

Every engine implements the `InferenceEngine` trait (`engine/mod.rs`):

```rust
#[async_trait]
pub trait InferenceEngine: Send + Sync {
    async fn start(&self, model_path: &str, context_size: u32, options: EngineStartOptions)
        -> Result<(u16, String), String>;   // Returns (port, api_token)
    async fn stop(&self) -> Result<(), String>;
    async fn is_ready(&self) -> bool;
    fn base_url(&self) -> Option<String>;   // e.g. http://127.0.0.1:{port}/v1
    fn model_id(&self) -> Option<String>;
    fn max_context(&self) -> Option<u32>;
    fn display_name(&self) -> &'static str;
    fn engine_id(&self) -> &'static str;
    fn uses_single_file_model(&self) -> bool;
    fn hf_search_tag(&self) -> &'static str; // "gguf", "mlx", "awq"
}
```

### 3.3 Engine Manager

`EngineManager` (Tauri managed state) holds the active engine instance:

- **Compile-time engine selection** — `create_engine()` uses `#[cfg(feature = "...")]` to instantiate exactly one
- **Priority order** — `mlx > vllm > llamacpp > ollama > none`
- **`uv` binary resolution** for MLX/vLLM — searches: dev `bin/`, exe dir, system PATH, `~/.scrappy/uv`
- **Bootstrap** — MLX and vLLM need first-launch setup (Python venv creation). Emits `engine_setup_progress` events.
- **Frontend auto-filtering** — `get_active_engine_info()` tells the frontend which HF Hub tag, model format, and engine name to display

### 3.4 Sidecar Manager

`SidecarManager` (`sidecar.rs`, 54KB) manages 4 sidecar processes alongside the engine:

| Sidecar | Port | Purpose |
|---------|------|---------|
| `llama-server` | Dynamic | LLM inference (legacy path, used when engine = llamacpp) |
| `whisper-server` | 53757 (fixed) | Speech-to-text (OpenAI-compatible API) |
| `embedding-server` | Dynamic | Local vector embeddings |
| `stable-diffusion-server` | Dynamic | Local image generation |

### 3.5 Inference Router (5 Modalities × N Backends)

The `InferenceRouter` (`inference/router.rs`, 21KB) routes every AI modality to either local sidecars or cloud providers:

**Per-modality cloud backends:**

| Modality | Cloud Backends |
|----------|----------------|
| **Chat** | Anthropic, OpenAI, Gemini, Groq, OpenRouter, Mistral, xAI, Together, Venice, Moonshot, MiniMax, NVIDIA NIM, Cohere, Xiaomi |
| **Embedding** | OpenAI, Gemini, Cohere, Voyage AI + local sidecar |
| **TTS** | OpenAI, ElevenLabs, Gemini + local Piper sidecar |
| **STT** | OpenAI, Deepgram, Gemini + local Whisper sidecar |
| **Diffusion** | Imagen 3 Flash/Pro (Gemini API), DALL-E 3, Stability AI, fal.ai FLUX, Together AI + local sd.cpp/mflux |

### 3.6 Cloud Chat Providers (14 Total)

All registered in `inference/provider_endpoints.rs`:

| Provider | Slug | API Compat | Default Model | Context |
|----------|------|------------|---------------|---------|
| Anthropic | `anthropic` | Anthropic | `claude-sonnet-4-20250514` | 200K |
| OpenAI | `openai` | OpenAI | `gpt-4o` | 128K |
| Google Gemini | `gemini` | Gemini | `gemini-2.5-flash` | 1M |
| Groq | `groq` | OpenAI-compat | `llama-3.3-70b-versatile` | 128K |
| OpenRouter | `openrouter` | OpenAI-compat | `anthropic/claude-sonnet-4` | 128K |
| Mistral AI | `mistral` | OpenAI-compat | `mistral-large-latest` | 128K |
| xAI (Grok) | `xai` | OpenAI-compat | `grok-3` | 131K |
| Together AI | `together` | OpenAI-compat | `Llama-3.3-70B-Instruct-Turbo` | 128K |
| Venice AI | `venice` | OpenAI-compat | `llama-3.3-70b` | 128K |
| Moonshot (Kimi) | `moonshot` | OpenAI-compat | `moonshot-v1-auto` | 128K |
| MiniMax | `minimax` | OpenAI-compat | `MiniMax-Text-01` | 1M |
| NVIDIA NIM | `nvidia` | OpenAI-compat | `meta/llama-3.3-70b-instruct` | 128K |
| Cohere | `cohere` | Cohere (v2) | `command-r-plus` | 128K |
| Xiaomi | `xiaomi` | OpenAI-compat | `MiMo-7B-RL` | 128K |

---

## 4. How Scrappy (UI — OpenClaw) Interfaces with IronClaw

### 4.1 Integration Architecture

```
┌───────────── Frontend (React/TypeScript) ──────────────┐
│                                                         │
│  OpenClawChatView.tsx ──listen("openclaw-event")───►    │
│          │                                        ▲     │
│          │ invoke("openclaw_send_message", {...})  │     │
│          ▼                                        │     │
├────── Tauri IPC (invoke → Result<T>, emit → events) ───┤
│                                                         │
│  rpc.rs   ──(1)─► IronClawState.activate_session()      │
│           ──(2)─► inject_tx.send(IncomingMessage)       │
│           ◄──────  returns Ok(()) immediately (~5ms)    │
│                                                         │
│  IronClaw Agent (background tokio task)                 │
│           ──(3)─► Agent::process_message()              │
│           ──(4)─► StatusUpdate → TauriChannel           │
│           ──(5)─► status_to_ui_event() mapping          │
│           ──(6)─► app.emit("openclaw-event", UiEvent)   │
│                            │                            │
└────────────────────────────┼────────────────────────────┘
                             ▼
                 Frontend event listener renders
```

**Communication contract:**
- **Commands (frontend → backend):** Tauri `invoke()` calls → `#[tauri::command]` functions in `rpc.rs`
- **Events (backend → frontend):** `app.emit("openclaw-event", UiEvent)` → `listen("openclaw-event")` in TypeScript
- **Agent turns:** Spawn-and-return pattern (5ms ACK, stream results via events)
- **Queries:** Direct `await` pattern (< 1ms, returns data immediately)

### 4.2 The Bridge Files

These 7 files form the complete integration layer between Scrappy and IronClaw:

| File | LOC | Purpose |
|------|-----|---------|
| **`ironclaw_bridge.rs`** | 698 | `IronClawState` — lifecycle management (start/stop), config resolution, `build_inner()` creates Agent + all services. Holds 6 backend service objects for the `tauri_commands` facade. |
| **`ironclaw_channel.rs`** | 272 | `TauriChannel` — implements `ironclaw::channels::Channel` trait. Two-tier session routing (metadata primary, timestamp-based fallback). Tray icon animation on activity. |
| **`ironclaw_types.rs`** | 134 | `status_to_ui_event()` — converts every IronClaw `StatusUpdate` variant to a frontend `UiEvent`. Exhaustive match on all 12 variants. |
| **`ironclaw_secrets.rs`** | 180 | `KeychainSecretsAdapter` — implements `ironclaw::secrets::SecretsStore` trait, bridging macOS Keychain to IronClaw's secrets API. |
| **`tool_bridge.rs`** | 343 | `TauriToolBridge` — routes hardware tool approval requests through Tauri's 3-tier UI (Deny / Allow Once / Allow Session), with session permission caching and 5-min timeout. |
| **`ui_types.rs`** | 167 | The stable frontend event contract — `UiEvent` enum with 15 variants, tagged via `#[serde(tag = "kind")]`. |
| **`sanitizer.rs`** | ~100 | LLM token stripping — removes `<\|im_start\|>`, `<\|end\|>`, and other model-specific tokens from output before displaying. |

### 4.3 Engine Lifecycle (Start / Stop / Restart)

```
App Launch → lib.rs::setup()
  → IronClawState::new_stopped(app_handle, state_dir)
  → IronClawState::start(secrets_store)
      → build_inner()
          ├── 1. Set env vars (DATABASE_BACKEND, LIBSQL_PATH, WHISPER_HTTP_ENDPOINT, etc.)
          ├── 1b. Set thinking env vars (AGENT_THINKING_ENABLED, AGENT_THINKING_BUDGET_TOKENS)
          ├── 1c. Detect LLM backend from Scrappy config:
          │       ├── local_inference_enabled? → sidecar port → LLM_BACKEND=openai_compatible
          │       ├── cloud brain selected? → provider endpoint → LLM_BACKEND={anthropic|openai|openai_compatible}
          │       └── fallback → LLM_BACKEND=ollama (safe, no API key needed)
          ├── 2. Load ironclaw.toml config (if exists), else env-only
          ├── 3. Create TauriChannel + TauriToolBridge
          ├── 4. AppBuilder → build_all() → components (db, llm, tools, workspace, etc.)
          ├── 5. ChannelManager → register TauriChannel
          ├── 6. Agent::new(config, deps, channels, heartbeat, hygiene, routines, context_manager)
          ├── 7. Agent::start_background_tasks() → bg_handle
          ├── 8. Create Sprint 13 service objects (CostTracker, CatalogCache, etc.)
          └── 9. Emit UiEvent::Connected { protocol: 2 }
```

**Stop lifecycle:**

```
IronClawState::stop()
  → Shutdown background tasks (self-repair, heartbeat, routines)
  → Shutdown all channels
  → Clear session-level tool permissions
  → Clear active session tracking
  → Remove LLM env vars (so next start re-detects backend)
  → Emit UiEvent::Disconnected
```

**Secret hot-reload (2-tier):**

```
IronClawState::reload_secrets(secrets_store)
  → Tier 1: ironclaw::api::config::refresh_secrets() — zero downtime
  → Tier 2 (fallback): stop() → start() — full restart
```

### 4.4 The TauriChannel — How Events Flow

`TauriChannel` implements `ironclaw::channels::Channel` with these trait methods:

| Trait Method | What It Does |
|-------------|--------------|
| `name()` | Returns `"tauri"` — the channel ID for routing |
| `start()` | Converts the `mpsc::Receiver` into a `MessageStream` (consumed once) |
| `respond(msg, response)` | Emits `UiEvent::AssistantFinal` with sanitized text, resets tray to idle |
| `send_status(status, metadata)` | Two-tier session routing → `status_to_ui_event()` → `app.emit()`. Animates tray on `Thinking`/`ToolStarted`. |
| `broadcast(user_id, response)` | Emits `AssistantFinal` with `session_key: "system"` (for heartbeat notifications) |
| `health_check()` | Always returns `Ok(())` — Tauri is always healthy while app is running |
| `shutdown()` | Logs shutdown, no-op |

**Session routing strategy (multi-session safe):**

1. **Primary:** Read `session_key` / `thread_id` from `StatusUpdate` metadata (injected by IronClaw's agent loop)
2. **Fallback:** If metadata doesn't contain a session key, use the most recently activated session from `active_sessions` (bounded `HashMap<String, u64>`, cap 32)

This replaced the old `Arc<RwLock<String>>` which was racy under concurrent sessions.

### 4.5 StatusUpdate → UiEvent Mapping

Every IronClaw `StatusUpdate` variant maps to a frontend `UiEvent` (exhaustive, no catch-all):

| StatusUpdate | UiEvent | Frontend Rendering |
|-------------|---------|-------------------|
| `Thinking(text)` | `AssistantInternal` | 🧠 thinking indicator |
| `StreamChunk(delta)` | `AssistantDelta` | Append text to current message |
| `ToolStarted { name }` | `ToolUpdate { status: "started" }` | Tool card with spinner |
| `ToolCompleted { name, success }` | `ToolUpdate { status: "ok"/"error" }` | Tool card with result |
| `ToolResult { name, preview }` | `ToolUpdate { status: "stream" }` | Tool output preview |
| `Status(text)` | `RunStatus` | Status bar update |
| `ApprovalNeeded { ... }` | `ApprovalRequested` | 3-tier approval card |
| `AuthRequired { ... }` | `WebLogin` | OAuth flow prompt |
| `AuthCompleted { ... }` | `WebLogin { status: "authenticated"/"failed" }` | Auth result |
| `JobStarted { ... }` | `CanvasUpdate` | Canvas panel with job info |
| `Error { message, code }` | `Error` | Error toast/banner |

### 4.6 The Frontend Event Contract (UiEvent)

The `UiEvent` enum (`ui_types.rs`) uses `#[serde(tag = "kind")]` for JSON serialization. The frontend pattern-matches on `kind`:

```typescript
// OpenClawChatView.tsx
listen<UiEvent>("openclaw-event", (event) => {
  switch (event.payload.kind) {
    case "AssistantDelta":    // Append streaming text
    case "AssistantFinal":    // Replace text, show usage
    case "AssistantInternal": // Show thinking indicator
    case "ToolUpdate":        // Render tool card
    case "ApprovalRequested": // Show approval dialog
    case "RunStatus":         // Update status bar
    case "Error":             // Show error
    // ... 15 variants total
  }
});
```

**15 UiEvent variants:** `Connected`, `Disconnected`, `SessionList`, `History`, `AssistantDelta`, `AssistantInternal`, `AssistantSnapshot`, `AssistantFinal`, `ToolUpdate`, `RunStatus`, `ApprovalRequested`, `ApprovalResolved`, `Error`, `WebLogin`, `CanvasUpdate`, `SubAgentUpdate`

### 4.7 What IronClaw Provides to Scrappy

Scrappy uses these IronClaw modules (compiled in as library crate):

| IronClaw Module | What Scrappy Uses |
|----------------|-------------------|
| `ironclaw::agent::Agent` | Core agent runtime — process messages, manage sessions, run tools |
| `ironclaw::channels::Channel` | Trait implemented by `TauriChannel` |
| `ironclaw::channels::ChannelManager` | Registers and manages the Tauri channel |
| `ironclaw::app::AppBuilder` | Builds all engine components (DB, LLM, tools, workspace, extensions, hooks, etc.) |
| `ironclaw::Config` | Configuration from env vars + TOML file |
| `ironclaw::secrets::SecretsStore` | Trait implemented by `KeychainSecretsAdapter` |
| `ironclaw::api::chat` | Send messages, abort, resolve approvals |
| `ironclaw::api::sessions` | List, delete, reset, spawn sessions |
| `ironclaw::api::memory` | Get/write workspace files (MEMORY.md, SOUL.md, etc.) |
| `ironclaw::api::skills` | List, toggle, install skills |
| `ironclaw::api::routines` | List, trigger, lint cron jobs |
| `ironclaw::api::config` | Get/set/patch config, refresh secrets |
| `ironclaw::api::system` | Status, diagnostics, health check |
| `ironclaw::tauri_commands` | Unified facade for 8 analytics/management commands |
| `ironclaw::llm::cost_tracker` | LLM cost tracking with daily/monthly aggregation |
| `ironclaw::extensions::clawhub` | Plugin registry search and install |
| `ironclaw::agent::routine_audit` | Routine execution history ring-buffer |
| `ironclaw::llm::response_cache_ext` | LLM response cache statistics |
| `ironclaw::extensions::lifecycle_hooks` | Plugin lifecycle event audit log |
| `ironclaw::extensions::manifest_validator` | Plugin manifest validation |

### 4.8 Backend Service Objects (Sprint 13)

These 6 IronClaw services are instantiated in `IronClawInner` during `build_inner()` and accessible via `IronClawState` accessor methods:

| Service | Wrapping | Initialized As | Accessor |
|---------|----------|----------------|----------|
| `CostTracker` | `Arc<RwLock<_>>` | `CostTracker::new(BudgetConfig::default())` | `.cost_tracker()` |
| `CatalogCache` | `Arc<RwLock<_>>` | `CatalogCache::new(3600)` — 1hr TTL | `.catalog_cache()` |
| `RoutineAuditLog` | `Arc<RwLock<_>>` | `RoutineAuditLog::new(500)` — 500-entry ring buffer | `.routine_audit_log()` |
| `CachedResponseStore` | `Arc<RwLock<_>>` | `CachedResponseStore::new(CacheConfig::default())` | `.response_cache()` |
| `AuditLogHook` | `Arc<_>` | `AuditLogHook::new()` — thread-safe, no mutable methods | `.audit_log_hook()` |
| `ManifestValidator` | `Arc<_>` | `ManifestValidator::new()` — stateless | `.manifest_validator()` |

**Wired commands using these services:**

| Command | Service | IronClaw Facade |
|---------|---------|-----------------|
| `openclaw_cost_summary` | CostTracker | `tauri_commands::cost_summary()` |
| `openclaw_cost_export_csv` | CostTracker | `tauri_commands::cost_export_csv()` |
| `openclaw_clawhub_search` | CatalogCache | `tauri_commands::clawhub_search()` |
| `openclaw_clawhub_install` | CatalogCache | `tauri_commands::clawhub_prepare_install()` |
| `openclaw_routine_audit_list` | RoutineAuditLog | `tauri_commands::routine_audit_list()` |
| `openclaw_cache_stats` | CachedResponseStore | `tauri_commands::cache_stats()` |
| `openclaw_plugin_lifecycle_list` | AuditLogHook | `tauri_commands::plugin_lifecycle_list()` |
| `openclaw_manifest_validate` | ManifestValidator | `tauri_commands::manifest_validate()` |
| `openclaw_cron_history` (bonus) | RoutineAuditLog | `tauri_commands::routine_audit_list()` |

---

## 5. Solidity Assessment — Is Everything Built Solid?

### 5.1 Bridge Layer Verdict: ✅ Solid

| Component | Assessment | Details |
|-----------|------------|---------|
| **Lifecycle (start/stop)** | ✅ Robust | `RwLock<Option<IronClawInner>>` pattern ensures clean start/stop. Graceful shutdown of background tasks, channels, permissions. LLM env vars cleared between restarts. |
| **Session routing** | ✅ Correct | Two-tier strategy (metadata-first, timestamp-fallback) replaced the racy single-variable approach. Bounded map (cap 32) prevents memory leaks. |
| **Event mapping** | ✅ Exhaustive | `status_to_ui_event()` handles all 12 `StatusUpdate` variants with no `_ =>` catch-all. Adding a new variant in IronClaw causes a compile error in Scrappy — enforced safety. |
| **Secrets** | ✅ Secure | macOS Keychain via `security-framework`. `KeychainSecretsAdapter` implements the `SecretsStore` trait. Hot-reload via 2-tier strategy (in-place or restart). |
| **Tool approvals** | ✅ Premium | 3-tier model (Deny / Once / Session) with 5-min session cache timeout and automatic eviction. Far better than IronClaw's binary approve/deny. |
| **Tray animation** | ✅ Polished | `set_tray_active()` with 3-second debounced auto-reset. Previous reset timer is properly aborted before scheduling new one. |
| **Database coexistence** | ✅ Patched | libsql-0.6.0 patched to handle `sqlite3_config()` `SQLITE_MISUSE` gracefully when sqlx calls it first. Both databases coexist without conflict. |
| **LLM backend detection** | ✅ Smart | Auto-detects from Scrappy config: local sidecar port → cloud brain → provider endpoint → safe Ollama fallback. No user intervention needed. |

### 5.2 Build System Verdict: ✅ Clean

- `cargo check` passes with **zero errors and zero new warnings**
- Only remaining warning: pre-existing `dead_code` in the ironclaw crate itself
- Build profiles optimized: `dev` uses `line-tables-only` debug info + `split-debuginfo = "unpacked"` to reduce target/ size
- Release uses `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"` for minimal binary

### 5.3 Type Safety Between IronClaw and Scrappy

Type conversions in `rpc.rs` handle all known mismatches:

| IronClaw Type | Scrappy Type | Conversion |
|--------------|-------------|------------|
| `BTreeMap` (CostSummary) | `HashMap` | `.into_iter().collect()` |
| `Option<f64>` (alert_threshold) | `f64` | `.unwrap_or(50.0)` |
| `usize` (cache stats) | `u64` | `as u64` cast |
| `f32` (hit rate) | `f64` | `as f64` cast |
| `RoutineOutcome` enum | `String` | Flat string mapping (`success`, `failure`, `skipped`, `timeout`) |
| `plugin` field | `plugin_id` field | Field rename during struct construction |

### 5.4 Known Limitations / Areas to Watch

| Area | Status | Notes |
|------|--------|-------|
| **Gmail OAuth PKCE** | 🔮 Deferred (Sprint 14) | Config exists, but PKCE browser flow not wired |
| **LLM routing rule builder** | 🔮 Deferred (Sprint 14) | Toggle API exists, full rule editor UI pending |
| **LLM env var safety** | ⚠️ `unsafe` blocks | `std::env::set_var` requires `unsafe` in Rust 2024 edition — used extensively in `build_inner()`. Functionally correct but technically UB if called concurrently with env reads from other threads. |
| **Cost tracking accuracy** | ⚠️ Standalone instance | `CostTracker` is instantiated fresh (`BudgetConfig::default()`), not wired to actual LLM request pipeline. Cost data relies on the facade, not live metrics. |
| **Session limit** | ℹ️ Bounded | Active sessions capped at 32. Oldest evicted when limit reached. Appropriate for desktop use. |

---

## 6. Scrappy-Exclusive Features (Not Part of IronClaw)

### 6.1 Image Generation (Imagine Studio)

| Component | Purpose |
|-----------|---------|
| `ImagineView.tsx` | Main view with generation controls |
| `ImagineGeneration.tsx` | Prompt → image pipeline |
| `ImagineGallery.tsx` | Persistent gallery with history |
| `ImagineSidebar.tsx` | Model/preset selector |
| `imagine.rs` (470 LOC) | Tauri command routing (local vs cloud) |
| `image_gen.rs` (884 LOC) | Local sd.cpp + mflux inference |
| `inference/diffusion/` | 5 cloud backends + local backend |

**Provider mapping (frontend → backend):**

| Provider ID | Backend File | Model |
|-------------|-------------|-------|
| `"nano-banana"` / `"gemini"` | `cloud_imagen.rs` | Imagen 3 Flash (Gemini API) |
| `"nano-banana-pro"` | `cloud_imagen.rs` | Imagen 3 Pro (Gemini API) |
| `"openai"` | `cloud_dalle.rs` | DALL-E 3 |
| `"stability"` | `cloud_stability.rs` | Stability AI SDXL |
| `"fal"` | `cloud_fal.rs` | fal.ai FLUX |
| `"together"` | `cloud_together.rs` | Together AI |
| `"local"` | `local.rs` / `image_gen.rs` | sd.cpp / mflux sidecar |

### 6.2 Other Scrappy-Only Features

| Feature | Component | Description |
|---------|-----------|-------------|
| **Model Library** | `ModelBrowser.tsx`, `model-library.ts` | 600+ pre-defined model configs |
| **HF Discovery** | `HFDiscovery.tsx` | Search GGUF/Safetensors on HuggingFace, one-click download |
| **Spotlight Bar** | `SpotlightBar.tsx` | `Cmd+Shift+K` floating quick-chat |
| **Voice Wake** | `VoiceWakeOverlay.tsx`, `use-voice-wake.ts` | VAD + Sherpa-ONNX keyword spotting → auto-record → transcribe |
| **Cloud Storage** | `cloud/` (21 files) | 7 providers: S3-compatible (AWS/R2/B2/GCS/MinIO/Wasabi), GDrive, Dropbox, OneDrive, iCloud, SFTP, WebDAV — with AES-256-GCM encryption |
| **RAG Pipeline** | `rag.rs` (40KB) | Document ingestion, chunking, usearch vector retrieval, ONNX reranking |
| **MCP Integration** | `McpTab.tsx`, `scrappy-mcp-tools/` | Model Context Protocol server management |
| **Persona System** | `PersonaTab.tsx` | Custom AI personalities |
| **Auto-Updates** | `UpdateChecker.tsx` | GitHub Releases + tauri-plugin-updater with signing key verification |
| **Onboarding** | `OnboardingWizard.tsx` | First-run setup flow |
| **Remote Deploy** | `RemoteDeployWizard.tsx` | Deploy agent to remote infrastructure |

---

## 7. Frontend Architecture

### 7.1 Component Inventory

```
frontend/src/
├── components/ (84+ files)
│   ├── chat/           (19) — Chat tab: bubbles, web search, streaming
│   ├── openclaw/       (34) — OpenClaw tab: 29 pages + fleet + canvas
│   ├── imagine/         (4) — Image generation studio
│   ├── settings/       (16) — Settings tabs
│   ├── navigation/      (2) — Sidebar + routing
│   ├── onboarding/      (1) — Setup wizard
│   ├── projects/        (2) — Project management
│   └── voice/           (1) — Voice wake overlay
├── hooks/              (11) — Custom React hooks
├── lib/                (13) — API wrappers, model library, utilities
└── tests/               (4) — Vitest test suites
```

### 7.2 OpenClaw UI Components (34 files)

| Component | Size | IronClaw Feature Surfaced |
|-----------|------|--------------------------|
| `OpenClawChatView.tsx` | 66KB | Full chat with streaming, tool cards, thinking mode, approval flow |
| `OpenClawPlugins.tsx` | 40KB | Plugin management + ClawHub search + lifecycle timeline |
| `CloudBrainConfigModal.tsx` | 31KB | 14-provider cloud model configuration |
| `OpenClawSidebar.tsx` | 26KB | Navigation + page routing |
| `OpenClawChannels.tsx` | 26KB | Channel config for Discord, Telegram, Signal, Slack, Webhook, Nostr |
| `OpenClawConfig.tsx` | 22KB | Settings editor with URL allowlist, compaction, failover |
| `OpenClawAutomations.tsx` | 19KB | Cron/routine management with lint validator + fire-times |
| `OpenClawSystemControl.tsx` | 19KB | Engine start/stop/status |
| `OpenClawSkills.tsx` | 19KB | Skill listing, toggle, install with trust gating |
| `RemoteDeployWizard.tsx` | 19KB | Remote deploy wizard |
| `LiveAgentStatus.tsx` | 18KB | Real-time tool execution, stuck loop detection |
| `OpenClawPairing.tsx` | 18KB | DM pairing codes for channel auth |
| `OpenClawMemory.tsx` | 17KB | Workspace files + hybrid search (BM25+vector) |
| `OpenClawBrain.tsx` | 16KB | AI brain configuration |
| `OpenClawChannelStatus.tsx` | 15KB | Per-channel state badges, uptime, message counters |
| `OpenClawHooks.tsx` | 15KB | Lifecycle hook listing and inspection |
| `OpenClawDashboard.tsx` | 14KB | System overview dashboard |
| `OpenClawCostDashboard.tsx` | 13KB | LLM cost tracking — daily/monthly/model/agent charts |
| `SubAgentPanel.tsx` | 13KB | Subagent spawning and monitoring |
| `OpenClawToolPolicies.tsx` | 12KB | Per-tool allow/deny policies |
| `OpenClawEventInspector.tsx` | 12KB | Live raw event stream with filters |
| `OpenClawPresence.tsx` | 11KB | Presence beacons and device tracking |
| `OpenClawRouting.tsx` | 10KB | Multi-agent routing configuration |
| `OpenClawRoutineAudit.tsx` | 9KB | Routine execution history with outcome filtering |
| `OpenClawDoctor.tsx` | 8KB | System health diagnostics |
| `ApprovalCard.tsx` | 7KB | 3-tier approval: Deny / Allow Once / Allow Session |
| `OpenClawCacheStats.tsx` | 6KB | Response cache hit rate and efficiency metrics |
| `MemoryEditor.tsx` | 6KB | SOUL.md, MEMORY.md, BOOTSTRAP.md editing |

---

## 8. Key Metrics

| Metric | Value |
|--------|-------|
| Backend Rust source | ~35 files, 20,000+ LOC |
| Frontend TypeScript | 84+ React components |
| Tauri commands (total) | 66+ |
| IronClaw bridge files | 7 files, ~1,900 LOC |
| Local inference engines | 4 (llamacpp, MLX, vLLM, Ollama) |
| Cloud chat providers | 14 |
| Cloud diffusion providers | 5 |
| Cloud embedding providers | 4 |
| Cloud TTS providers | 3 |
| Cloud STT providers | 3 |
| Messaging channels (IronClaw) | 13 |
| Build warnings | **0** |
| Build errors | **0** |
| Time-to-first-token (in-process) | ~50ms (was ~200ms with WS bridge) |
| Dead code removed (integration) | ~2,458 LOC |
| Disk recovered (Node.js removal) | ~625 MB |

---

## 9. Technology Stack

| Layer | Technology |
|-------|-----------|
| Desktop Framework | Tauri v2 |
| Backend Language | Rust (Edition 2021, MSRV 1.92+) |
| Frontend | React + TypeScript + Vite |
| Agent Engine | IronClaw (in-process library crate, `features = ["desktop"]`) |
| Chat Engine | rig-core 0.7.0 (Rust) |
| Local Inference | llama.cpp / MLX / vLLM / Ollama (compile-time selection) |
| Image Generation | sd.cpp, mflux (local) + Imagen 3, DALL-E 3, Stability AI, fal.ai, Together (cloud) |
| Speech | Whisper (STT), Piper/ElevenLabs/OpenAI (TTS), Sherpa-ONNX (wake word) |
| Vector Search | usearch + ONNX reranking (ort 2.0.0-rc.9) |
| Database | SQLite (sqlx 0.7) + libSQL (IronClaw, patched for coexistence) |
| Secrets | macOS Keychain (security-framework 2) |
| Type Generation | Specta (Rust → TypeScript bindings via tauri-specta) |
| Cloud Storage | OpenDAL 0.55 (S3, WebDAV, SFTP) + AES-GCM encryption |
| Browser Automation | chromiumoxide 0.8.0 |
| Testing | Vitest (frontend) + built-in (Rust) |
| Scripting | Rhai (tool orchestration in Chat tab) |
| MCP | scrappy-mcp-tools (local path dependency) |
