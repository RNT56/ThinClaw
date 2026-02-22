# Scrappy — Outstanding Issues & Tech Debt

> Generated from `TECHNICAL_ARCHITECTURE.md`, `MICROSERVICES_AND_SIDECARS.md`, `FRONTEND_ARCHITECTURE.md`  
> Last updated: 2026-02-22

---

## 🔴 Critical

- [x] **Hardcoded developer path in `tauri.conf.json`** *(fixed 2026-02-22)*  
  Removed `/Users/mt/Library/...` — asset scope now uses `$APP_DATA` and `$HOME` dynamic variables only.  
  → `src-tauri/tauri.conf.json`

---

## 🟠 Dead Code / Cleanup

- [x] **Delete `commands_old.rs`** (105 KB) *(deleted 2026-02-22)*  
  Pre-refactor monolith removed.

- [x] **Delete `config_old.rs`** (77 KB) *(deleted 2026-02-22)*  
  Pre-refactor monolith removed.

---

## 🟡 Architecture & Design

- [x] **Split `ChatLayout.tsx` monolith** *(done 2026-02-22)*  
  Now a ~75-line shell. State lives in `ChatProvider.tsx` (~450 lines), views in `chat/views/`, sidebars in `chat/sidebars/`.

- [x] **Cache `RigManager` across requests** *(done 2026-02-22)*  
  `rig_cache::RigManagerCache` (new Tauri-managed state) stores the last built `RigManager` alongside a `RigManagerKey`.  
  `chat_stream` calls `cache.get_or_build(key, || RigManager::new(…))` — the manager is only rebuilt when provider, model, token, context size, tools, or knowledge content changes.  
  → `src-tauri/src/rig_cache.rs` (new), `src-tauri/src/chat.rs`, `src-tauri/src/lib.rs`

<!-- ─── Open items in recommended implementation order ─── -->

- [x] **① Deharcode `ToolRouter` host-tool dispatch** *(done 2026-02-22)*  
  Replaced hardcoded `match` with registry-driven dispatch: `ToolRouter::host_tool_names()` derives the set from `tool_discovery::get_host_tools_definitions()`. Adding a new host tool in one place auto-routes it everywhere.  
  → `src-tauri/src/rig_lib/tool_router.rs`

- [x] **② Wire tool events from `McpRequestHandler` to OpenClaw engine** *(done 2026-02-22)*  
  Replaced `SilentReporter` with `TauriEventReporter` that emits `ToolEvent`s as structured `"tool_event"` Tauri events (kind, message, tool_name, percentage, status) to the frontend.  
  → `src-tauri/src/openclaw/ipc.rs`

- [x] **③ Unify legacy `<tool_code>` XML loop with MCP sandbox loop** *(done 2026-02-22)*  
  Deleted `run_legacy_tool_loop` (~490 lines). `run_turn` now always uses the sandbox/Rhai path via `build_sandbox_unconditional()`, which ensures a sandbox is available for local host tools even without a remote MCP server. `orchestrator.rs` shrunk from 1299 → ~560 lines.  
  → `src-tauri/src/rig_lib/orchestrator.rs`

- [x] **④ Unify dual inference paths — extract `resolve_provider()`** *(done 2026-02-22)*  
  Extracted the shared 70-line provider-routing `match` block into a single `pub async fn resolve_provider()` free function plus a `ProviderConfig` struct in `chat.rs`.  
  Both `chat_stream` and `chat_completion` now call `resolve_provider()` — adding a new provider requires editing exactly one place.  
  The full `Box<dyn InferenceBackend>` trait (as originally envisioned) is deferred until the orchestrator's streaming API is stable.  
  → `src-tauri/src/chat.rs`

---

## 🟡 Missing / Incomplete Features

- [x] **Expose MCP server URL in settings UI** *(fixed 2026-02-22)*  
  `mcp_base_url` / `mcp_auth_token` now loaded from live `ConfigManager` in `get_mcp_config()`.  
  → `src-tauri/src/openclaw/ipc.rs`  
  > **Note:** Settings UI fields in `GatewayTab.tsx` already exist and write to `UserConfig` — the backend now reads them correctly.

- [x] **Implement TTS sidecar** *(done 2026-02-22)*  
  Implemented Piper-based TTS sidecar in `src-tauri/src/tts.rs`. `tts_synthesize` Tauri command spawns
  `piper` binary via stdin→stdout, returns base64 PCM to frontend. "Read Aloud" button added to
  assistant `MessageBubble` using Web Audio API. `bindings.ts` updated with `ttsSynthesize` binding.  
  Binary must be bundled at `src-tauri/bin/piper` — see `documentation/latest/MICROSERVICES_AND_SIDECARS.md`.  
  → `src-tauri/src/tts.rs`, `src-tauri/src/sidecar.rs`, `src/components/chat/MessageBubble.tsx`

- [ ] **Add auth to whisper-server** ⛔ *Blocked on upstream*  
  `whisper-server` has no `--api-key` flag. The STT endpoint is unauthenticated on localhost and will
  remain so until `whisper.cpp` adds server auth support. Track: https://github.com/ggml-org/whisper.cpp  
  **No implementation is possible** without forking whisper.cpp or waiting for upstream.  
  Workaround: STT traffic stays on localhost-only (127.0.0.1); not network-exposed by default.  
  → `src-tauri/src/sidecar.rs`

---

## 🟡 Type Safety & Correctness

- [x] **Add runtime null guards over `bindings.ts`** *(done 2026-02-22)*  
  Removed `// @ts-nocheck` from `bindings.ts` (and from the specta generator config in `lib.rs`).  
  Created `src/lib/guards.ts` with `defined()`, `withDefault()`, and `unwrapResult()` helpers.  
  Fixed the `TAURI_CHANNEL` name collision (generated placeholder vs. runtime import).  
  → `src/lib/bindings.ts`, `src/lib/guards.ts`, `src-tauri/src/lib.rs`

---

## 🟢 Performance / Minor

- [x] **Lazy-load heavy settings tabs** *(done 2026-02-22)*  
  `SecretsTab` (64 KB), `GatewayTab` (68 KB), `ModelBrowser` (63 KB) are now `React.lazy()` with `<Suspense>` fallbacks.  
  → `src/components/settings/SettingsPages.tsx`

- [x] **Make config writes async** *(done 2026-02-22)*  
  `ConfigManager::save_config()` now spawns a `tokio::fs::write` task, keeping the in-memory Mutex update synchronous while deferring disk I/O.  
  → `src-tauri/src/config.rs`

- [x] **Make `ToolRegistryCache` TTL configurable** *(done 2026-02-22)*  
  Added `mcp_cache_ttl_secs: u64` to `UserConfig` (default: 300 s). Ready for wiring when cache is instantiated in `tool_discovery.rs`.  
  → `src-tauri/src/config.rs`, `src-tauri/scrappy-mcp-tools/src/discovery.rs`

- [x] **Make `summarize_result` truncation limit configurable** *(done 2026-02-22)*  
  Added `mcp_tool_result_max_chars: usize` to `UserConfig` (default: 5000). `McpRequestHandler::call_tool` now reads this value from `ConfigManager`.  
  → `src-tauri/src/openclaw/ipc.rs`, `src-tauri/src/config.rs`

---

## 🔵 Repository Structure — Planned Refactor

- [x] **Reorganise project into `backend/` + `frontend/` top-level folders** *(done 2026-02-22)*  
  The current mixed layout (Rust sources in `src-tauri/`, React sources in `src/` at root) makes the repository hard to navigate.  
  Goal: move everything into two clearly-named siblings.

  **Proposed new layout:**
  ```
  scrappy/
  ├── backend/              ← rename of src-tauri/
  │   ├── Cargo.toml
  │   ├── build.rs
  │   ├── tauri.conf.json
  │   ├── capabilities/
  │   ├── icons/
  │   ├── resources/
  │   ├── scrappy-mcp-tools/
  │   └── src/
  ├── frontend/             ← new folder containing all web assets
  │   ├── src/              ← current root src/
  │   ├── public/           ← current root public/
  │   ├── index.html        ← current root index.html
  │   ├── vite.config.ts
  │   ├── vitest.config.ts
  │   ├── tsconfig.json
  │   ├── tsconfig.node.json
  │   ├── tsconfig.test.json
  │   ├── tailwind.config.cjs
  │   └── postcss.config.js
  ├── documentation/
  ├── package.json          ← root package.json stays (Tauri CLI entry point)
  └── ...
  ```

  **Files that need updating after the move:**
  | File | Change required |
  |---|---|
  | `backend/tauri.conf.json` | `"frontendDist": "../../frontend/dist"`, `"devUrl"` same |
  | `backend/tauri.conf.json` `build` section | `"beforeDevCommand"` / `"beforeBuildCommand"` must call into `frontend/` |
  | Root `package.json` | `scripts.dev` / `scripts.build` → `cd frontend && npm run …` |
  | `frontend/vite.config.ts` | `watch.ignored` → `["**/backend/**"]` |
  | `frontend/tsconfig.json` | `"include": ["src"]` — unchanged (relative to `frontend/`) |
  | `.gitignore` | Update `src-tauri/target` → `backend/target` |
  | CI / Dockerfile | Update any path references to `src-tauri` |

  **Key gotchas:**
  - Tauri's `tauri.conf.json` paths are resolved relative to the **config file's location**, not the workspace root. Double-check `frontendDist` and `"devUrl"` after move.
  - The `tauri-build` build script (`build.rs`) must stay in `backend/` at the crate root — no change needed.  
  - `scrappy-mcp-tools` is a workspace sub-crate; its path in `backend/Cargo.toml` is already relative (`path = "scrappy-mcp-tools"`) — no change needed.
  - The Tauri CLI (`npm run tauri`) must be run from the **root** `package.json`, not from `frontend/`.
  - Any VS Code launch configs (`.vscode/`) referencing `src-tauri` need updating.
  - Run `cargo check` from `backend/` and `npm run build` from `frontend/` independently before running `tauri build` to catch any residual path issues.
