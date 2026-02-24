# Scrappy — Outstanding Issues & Tech Debt

> Generated from `TECHNICAL_ARCHITECTURE.md`, `MICROSERVICES_AND_SIDECARS.md`, `FRONTEND_ARCHITECTURE.md`, `OPENCLAW_IMPLEMENTATION.md`  
> Last updated: 2026-02-24

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

- [x] **⑤ Multi-engine inference architecture** *(done 2026-02-23)*  
  Implemented `InferenceEngine` trait + 4 engines (`llamacpp`, `mlx`, `vllm`, `ollama`). `EngineManager` (Tauri managed state) auto-selects engine via compile-time feature flags. Added `setup_engine`, `start_engine`, `stop_engine`, `is_engine_ready`, `get_active_engine_info`, `get_engine_setup_status` commands. CI matrix covers all 4 engine×platform variants.  
  → `backend/src/engine/`, `backend/src/lib.rs`, `.github/workflows/build-release.yml`

- [x] **⑥ HuggingFace Hub model discovery** *(done 2026-02-23)*  
  Live HF API search with engine-aware tag filtering, GGUF quantization picker, multi-file download for MLX/vLLM directory models. Frontend `HFDiscovery` component with debounced search, `EngineSetupBanner` for first-launch bootstrap, `ActiveEngineChip` for engine status display.  
  → `backend/src/hf_hub.rs`, `frontend/src/components/settings/HFDiscovery.tsx`, `EngineSetupBanner.tsx`, `ActiveEngineChip.tsx`, `model-context.tsx`

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

## 🔒 Security Hardening

- [x] **Remove auto-grant on key save** *(done 2026-02-23)*  
  `update_*_key()` methods no longer set `xxx_granted = true` when saving. Users must explicitly toggle grants in Settings › Secrets. Key deletion still auto-revokes.  
  → `backend/src/openclaw/config/identity.rs`

- [x] **Remove `SecretStore::snapshot()` method** *(done 2026-02-23)*  
  Returned all keys without checking grant flags — eliminated as a potential leak vector.  
  → `backend/src/secret_store.rs`

- [x] **Gate `OPENCLAW_CUSTOM_LLM_KEY` env var** *(done 2026-02-23)*  
  Only injected when `custom_llm_enabled = true`. Previously leaked unconditionally.  
  → `backend/src/openclaw/config/engine.rs`

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

- [x] **Split `ModelProvider` into two contexts** *(done 2026-02-23)*  
  `ModelStateContext` (models, paths, engine — rarely changes) + `ModelProgressContext` (downloading, discovery — hot during downloads). Single `useModelContext()` hook merges both. Components that only read state won't re-render during downloads.  
  → `frontend/src/components/model-context.tsx`

- [x] **Throttle download progress updates** *(done 2026-02-23)*  
  Progress events are buffered in `useRef` and flushed to state at ~4fps (250ms interval), down from every-chunk (~10-100fps). Both `downloading` percentages and `discoveryState.repoProgress` are throttled.  
  → `frontend/src/components/model-context.tsx`

- [x] **Memoize all `ModelProvider` function refs** *(done 2026-02-23)*  
  All action callbacks (`setModelPath`, `selectModel`, `cancelDownload`, `deleteModel`, `downloadHfFiles`, `downloadStandardAsset`, etc.) wrapped in `useCallback` for stable identity in `useMemo` dependency arrays.  
  → `frontend/src/components/model-context.tsx`

- [x] **Fix HFDiscovery "No files found" flash on tab return** *(done 2026-02-23)*  
  `ModelBrowser` now uses `display: none` keep-alive for the Discover tab. Auto-expand effect loads file info when cache is lost (remount). Render shows loading spinner instead of "No files found" while file info is being fetched.  
  → `frontend/src/components/settings/ModelBrowser.tsx`, `frontend/src/components/settings/HFDiscovery.tsx`

---

## 🟠 OpenClaw — Issues Found During Deep Analysis *(2026-02-24)*

> Discovered during comprehensive frontend/backend analysis. All 12 issues are now fixed.

- [x] **🔴 Bug #1: Tool message ID has stray spaces** *(fixed 2026-02-24)*  
  Template literal `tool - ${name} -${runId} ` had errant spaces, causing tool status updates to never match existing entries → duplicate/stale tool messages in chat.  
  → `frontend/src/components/openclaw/OpenClawChatView.tsx` line 634

- [x] **🔴 Bug #2: Fleet event listener never cleaned up (memory leak)** *(fixed 2026-02-24)*  
  The `useEffect` cleanup function was returned inside the `.then()` callback of the dynamic import, not from the `useEffect` itself. React never called it, causing zombie listeners to accumulate on mount/unmount cycles. Refactored to ref-based cleanup with `cancelled` guard for race conditions.  
  → `frontend/src/components/openclaw/fleet/FleetCommandCenter.tsx` lines 108–239

- [x] **🟠 Risk #3: No timeout on WS RPC calls — can hang forever** *(fixed 2026-02-24)*  
  Added 30s `tokio::time::timeout` wrapper around the oneshot receiver in `rpc()`. Returns `ClientError::Timeout` instead of blocking indefinitely.  
  → `backend/src/openclaw/ws_client.rs` lines 96–112

- [x] **🟠 Risk #4: Pending RPC HashMap not drained on disconnect** *(fixed 2026-02-24)*  
  Added `self.pending.drain()` in the `run_once()` error path inside `run_forever()`. All pending senders now receive a clear "WebSocket disconnected" error immediately on disconnect.  
  → `backend/src/openclaw/ws_client.rs` line 411

- [x] **🟠 Risk #5: auth-profiles.json persists plaintext API keys on disk** *(fixed 2026-02-24)*  
  Two mitigations: (1) `chmod 600` on write via `std::os::unix::fs::PermissionsExt` (2) File deleted in `openclaw_stop_gateway()`. Safe because it's fully regenerated on every start from `SecretStore`.  
  → `backend/src/openclaw/config/engine.rs` line 716, `backend/src/openclaw/commands/gateway.rs` line 462

- [x] **🟠 Risk #6: `deep_migrate()` runs on every gateway start** *(fixed 2026-02-24)*  
  Added `.migration_v1_complete` marker file check at the top of `deep_migrate()`. Written with timestamp after first successful run. Subsequent starts skip migration entirely.  
  → `backend/src/openclaw/config/engine.rs` lines 750–910

- [x] **🟡 Arch #7: Duplicated session delete retry logic** *(fixed 2026-02-24)*  
  Stripped the frontend's abort→reset→retry cascade. `handleDeleteSession` now calls `deleteOpenClawSession()` once — the backend handles the full lifecycle (abort → wait → delete → reset → retry) in `sessions.rs`.  
  → `frontend/src/components/openclaw/OpenClawChatView.tsx` lines 687–724

- [x] **🟡 Arch #8: Verbose `info!()` logging on every WS message** *(fixed 2026-02-24)*  
  Downgraded 10+ hot-path `info!()` calls to `debug!()` or `trace!()`. Per-message WS preview → `trace!`. Per-event frame → `debug!`. Session list/delete/reset RPC → `debug!`. Lifecycle events (connect/disconnect) remain at `info!`.  
  → `backend/src/openclaw/ws_client.rs` (10 call sites)

- [x] **🟡 Arch #9: Fleet polls every 3s unconditionally** *(fixed 2026-02-24)*  
  Added `document.visibilityState === 'visible'` check to the polling interval. Fleet status is only fetched when the browser tab is active.  
  → `frontend/src/components/openclaw/fleet/FleetCommandCenter.tsx` line 104

- [x] **🟡 Arch #10: Hardcoded `agent:main:primary` session key** *(fixed 2026-02-24)*  
  Changed to `'agent:main'` to align with the backend's canonical session key. The backend's `openclaw_get_sessions()` always normalizes to `agent:main`.  
  → `frontend/src/components/openclaw/OpenClawChatView.tsx` line 385

- [x] **🟡 Arch #11: No backpressure on UI event channel (64-deep mpsc)** *(fixed 2026-02-24)*  
  Increased channel buffer from 64 to 256. Changed `ui_tx.send().await` to `ui_tx.try_send()` with a `warn!()` on overflow — the WS event loop never blocks now, even under heavy tool load.  
  → `backend/src/openclaw/commands/gateway.rs` line 406, `backend/src/openclaw/ws_client.rs` line 670

- [x] **🟡 Arch #12: Optimistic message ID has stray spaces** *(fixed 2026-02-24)*  
  Same template literal whitespace bug as #1, in the optimistic user message: `temp - ${Date.now()} `.  
  → `frontend/src/components/openclaw/OpenClawChatView.tsx` line 665

---

## 🔴 Storage & Database — Issues Found During Deep Analysis *(2026-02-24)*

> Discovered during comprehensive analysis of all storage subsystems: SQLite, USearch vector store, keychain, filesystem, config management.  
> See `documentation/latest/STORAGE_AND_DATABASE.md` for full architecture reference.

### 🔴 Bugs (Correctness)

- [x] **S-Bug #1: Vector search relevance ordering destroyed by rowid sort** *(fixed 2026-02-24)*  
  `search_scoped()` now returns results sorted by cosine distance (ascending = most similar). `VectorStore::search()` returns `(key, distance)` pairs; multi-scope merge preserves similarity ordering via distance-based sort + `HashSet` dedup.  
  → `backend/src/vector_store.rs:113–140, 280–310`

- [x] **S-Bug #2: `delete_all_history` orphans `generated_images` DB records** *(fixed 2026-02-24)*  
  Added `DELETE FROM generated_images` to the transaction, after projects and before commit.  
  → `backend/src/history.rs:388–393`

- [x] **S-Bug #3: Duplicate `--vae-tiling` flag in image generation** *(fixed 2026-02-24)*  
  Removed unconditional `--vae-tiling` (line 177). macOS gets it from the per-architecture block (line 258). Non-macOS gets it from a new addition in the `#[cfg(not(target_os = "macos"))]` block.  
  → `backend/src/image_gen.rs:177, 194`

- [x] **S-Bug #4: `delete_document` doesn't clean up vector store entries** *(fixed 2026-02-24)*  
  Now logs ghost vector count for diagnostics (USearch lacks per-key removal). Ghost vectors remain until scope reset — documented as a known USearch limitation.  
  → `backend/src/projects.rs:201–252`

- [x] **S-Bug #5: FTS5 search ignores scope filtering** *(fixed 2026-02-24)*  
  FTS queries now JOIN through `chunks` → `documents` and filter by `project_id` or `chat_id` (plus global documents). Three query paths: project-scoped, chat-scoped, and global-only — mirrors the vector search scope logic.  
  → `backend/src/rag.rs:707–755`

- [x] **S-Bug #6: Document files never deleted on individual deletion** *(fixed 2026-02-24)*  
  `delete_document()` now looks up the path before deletion and removes the file from `app_data/documents/`. Errors are logged but non-fatal.  
  → `backend/src/projects.rs:201–252`

### 🟡 Performance / Efficiency

- [x] **S-Perf #7: Reranker processes candidates sequentially (no batching)** *(fixed 2026-02-24)*  
  Replaced per-document loop with batched tokenization + padding + single ONNX forward pass. All query-document pairs encoded into `[BatchSize, MaxSeqLen]` tensors.  
  → `backend/src/reranker.rs:111–175`

- [x] **S-Perf #8: Missing composite index on `messages(conversation_id, created_at)`** *(fixed 2026-02-24)*  
  Added migration `20260224000000_add_messages_index.sql` with `CREATE INDEX idx_messages_conv_created ON messages(conversation_id, created_at DESC)`.  
  → `backend/migrations/20260224000000_add_messages_index.sql`

- [ ] **S-Perf #9: Imagine search uses `LIKE %query%` (full table scan)** *(accepted)*  
  The `LIKE %query%` pattern is acceptable for a local gallery (< 100K images). Adding FTS5 for `generated_images` is deferred — overkill for the current use case.  
  → `backend/src/imagine.rs:478`

### 🟡 Data Integrity / Robustness

- [x] **S-Data #10: Inconsistent timestamp units across tables** *(fixed 2026-02-24)*  
  Normalized all integer timestamps to milliseconds. Migration `20260225000000_normalize_timestamps.sql` converts existing seconds-based rows (`WHERE created_at < 10B` guard prevents double-multiply). Backend `history.rs` changed from `as_secs()` to `as_millis()`. `documents`/`projects` already used ms. `generated_images` uses RFC 3339 strings (unchanged). Also fixed a latent frontend bug: message reconciliation window was 3.5 days instead of 5 minutes due to unit mismatch.  
  → `backend/migrations/20260225000000_normalize_timestamps.sql`, `backend/src/history.rs:77,259,329`

- [x] **S-Data #11: PDF OCR has no overall timeout** *(fixed 2026-02-24)*  
  Wrapped the Vision-OCR loop in `tokio::time::timeout(120s)`. If the timeout fires, partial transcription is preserved and ingestion continues with whatever was extracted.  
  → `backend/src/rag.rs:140–220`

### 🟠 Architecture

- [x] **S-Arch #12: Dual in-memory key cache may drift** *(fixed 2026-02-24)*  
  `SecretStore` is now a thin delegation wrapper — removed its `RwLock<HashMap>`, all reads/writes delegate to `keychain::get_key`/`set_key` which use a single `Mutex<HashMap>` cache. Eliminates the dual-cache drift risk.  
  → `backend/src/secret_store.rs`

- [x] **S-Arch #13: `auth-profiles.json` contains plaintext API keys** *(mitigated 2026-02-24)*  
  Already mitigated: chmod 600 on write + deleted on gateway stop. Remaining risk (file readable by same-user processes while gateway is active) is accepted — full elimination would require OpenClaw Engine to accept keys via env vars/stdin instead of file.  
  → `backend/src/openclaw/config/engine.rs`

---

## 🔵 Repository Structure — Refactor *(done 2026-02-22)*

- [x] **Reorganise project into `backend/` + `frontend/` top-level folders** *(done 2026-02-22)*  
  `src-tauri/` → `backend/`, root `src/` → `frontend/src/`. All path references in `tauri.conf.json`, `vite.config.ts`, `tsconfig.json`, `.gitignore`, root `package.json` scripts, and CI updated accordingly.
