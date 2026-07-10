# WS-04 — Desktop App Completion (cloud-sync, inference, dual stacks)

> **Status:** ✅ Landed (2026-06-24), commit `41091179` (Wave 1: desktop app completion, cloud-sync + S3 metadata + orchestration). Tasks T1 through T11 shipped (live cloud sync spawned on migrate + startup and stopped on exit; `last_modified` fixed in all three opendal providers; the dead `InferenceRouter` chat modality erased; `sub_agent_registry` eviction wired; `sandbox_factory` tool-parse resolved; `image_gen` divide-by-zero guarded; the dual-stack doc addendum added). **Still open:** **T12** (the desktop build profile does not enable `wasm-runtime` and carries no explicit inline rationale). This plan is otherwise complete; do not re-execute the landed tasks.
> **Priority:** P1 · **Risk:** high · **Effort:** L
> **Depends on:** none · **Blocks:** WS-10 (desktop god-file decomposition coordinates with the small local splits here)
> **Owns (symbols/files):** Everything under `apps/desktop/backend/src/cloud/**`, `apps/desktop/backend/src/file_store.rs`, `apps/desktop/backend/src/inference/**`, `apps/desktop/backend/src/rig_lib/sandbox_factory.rs` (tool-parse TODOs only), `apps/desktop/backend/src/image_gen.rs:700` (progress divide-by-zero label), the desktop build profile in the root `Cargo.toml` `[features] desktop = [...]` (wasm-runtime omission), the `sub_agent_registry` module in `apps/desktop/backend/src/thinclaw/commands/rpc_orchestration.rs`, and `apps/desktop/documentation/runtime-boundaries.md` (dual-stack documentation addendum). The `apps/desktop/backend` package is workspace-excluded with its own `sqlx` 0.8 — no other WS may edit these files.

## Vision & Goal

ThinClaw Desktop's promise is a private, local-first agent that the operator can *optionally* mirror to client-side-encrypted cloud storage and recover on a new device. The cloud-sync subsystem is fully written (encryption, manifests, providers, snapshot, network adaptation, App Nap guard, progress events) but is **never spawned** after migration — once you switch to cloud mode, new writes stay local and a fresh device cannot pull them. This workstream realizes that vision: it activates the live sync path so "Cloud mode" actually means cloud mode, fixes a latent conflict-resolution data bug, resolves the dead `InferenceRouter` chat path, documents the intentional dual-agent split so it stops reading as drift, and closes a slow child-session memory leak whose doc comment already lies about being fixed.

## Scope

**In scope:**
- Activate end-to-end cloud sync on `migrate_to_cloud` success: `FileStore` cloud mode + upload worker draining `UploadJob`, `SyncEngine::run` gated by `network::recommend_strategy` + `AppNapGuard`, the read-path cloud download fallback, and startup restoration of all of the above (`apps/desktop/backend/src/cloud/mod.rs`, `file_store.rs`, `cloud/sync.rs`, `cloud/network.rs`, `cloud/app_nap.rs`, `lib.rs`).
- Fix `last_modified = 0` in the three opendal-backed providers (`cloud/providers/s3.rs`, `webdav.rs`, `sftp.rs`).
- Resolve the dead `InferenceRouter` chat backend + local-backend lazy-set path (wire-or-remove decision).
- Document the intentional dual-stack split's two-MCP-client / two-provider-builder consequence in `runtime-boundaries.md`.
- Wire `sub_agent_registry::remove_parent` / `clear` to session-delete and engine-stop.
- Resolve the two `sandbox_factory.rs` tool-parse TODOs.
- Guard the `image_gen.rs:700` divide-by-zero progress `%` label (display-only, no crash).
- Resolve the desktop build profile omitting the `wasm-runtime` feature in the root `Cargo.toml` `[features] desktop = [...]` (add it or document the omission, coordinated with WS-12).

**Out of scope (and which WS owns it):**
- Heavy decomposition of `rpc_dashboard.rs` (2332L), `remote_proxy.rs` (1787L), `sidecar.rs` (1710L) — owned by **WS-10**. Small local splits of files this WS already touches (e.g. carving an upload-worker submodule out of `cloud/`) are allowed here.
- Any root-crate (`thinclaw_core`/`thinclaw_*`) change. The dual-stack consolidation explicitly does **not** merge the two runtimes (see Decision Points).

## Current State (verified)

Cloud-sync subsystem — **FIXED (wired end-to-end):**
- The "never spawned after migration" premise no longer holds. A new `cloud/live_sync.rs` module implements `start_live_sync` (spawns the upload worker draining `UploadJob` + the `SyncEngine`) and a `SyncHandles` bundle installed via `CloudManager::install_sync_handles` (`cloud/mod.rs:358`), with `stop_sync` (`cloud/mod.rs:371`) to cancel/abort them.
- `start_live_sync` is invoked on migrate-to-cloud success (`cloud/commands.rs:343`) and on startup when already in cloud mode (`lib.rs:509`); `migrate_to_local`/exit stop it (`cloud/commands.rs:377`, `lib.rs:676` on `RunEvent::Exit`). `CloudManager` now exposes `active_provider()` (`cloud/mod.rs:337`) and `master_key()` (`:342`) so the worker can reach the live provider + key without widening `CloudManagerInner`.
- The upload worker reuses the migration encrypt/key convention (`encryption::encrypt(master_key, relative_path, data)` → `provider.put("{relative_path}.enc", ..)`) and honors `SyncStrategy`/`AppNapGuard`; `FileStore` cloud mode + upload channel are set through the worker, and the read path downloads-and-caches instead of erroring. `SyncEngine`, `network::recommend_strategy`, and `AppNapGuard` now have production callers.

S3 `last_modified` latent bug — **FIXED in all three opendal providers:**
- The hardcoded `last_modified: 0` is gone from all three opendal-backed providers: `cloud/providers/s3.rs:142-143` reads `meta.last_modified()`, and `webdav.rs:136` / `sftp.rs:131` use `m.last_modified().map(opendal_timestamp_millis).unwrap_or(0)`. The native-API providers already populated it correctly. No hardcoded `last_modified: 0` remains outside test fixtures.

InferenceRouter dead chat path — **FIXED (erased, Decision #2):**
- The tts/stt/diffusion/embedding modalities remain wired. The dead chat modality has been removed: `router.rs` no longer stores an `Arc<dyn ChatBackend>`, `clear_backend(Modality::Chat)` is a documented no-op (`inference/router.rs:137`), `reconfigure()` constructs nothing for chat ("Nothing to construct here", `router.rs:318`), and the `inference/chat/` directory (`local.rs`/`cloud.rs`/`mod.rs`) is deleted. The real chat path is still `chat.rs::direct_chat_stream` → `resolve_provider`. Only a config-derived `active_chat_backend()` settings-UI badge (`router.rs:155`) remains.

Dual desktop agent stacks — **intentional and already documented:**
- Two systems are explicitly specified in `apps/desktop/documentation/runtime-boundaries.md` (updated 2026-05-15): **System A: Direct AI Workbench** (`chat.rs`, `rig_lib/*`, `inference/*`, `sidecar.rs`) and **System B: ThinClaw Agent Cockpit** (`thinclaw/*` over embedded `thinclaw_core`). The doc covers surfaces, ownership, persistence, and security boundaries.
- Consequence now called out in the doc (T10 landed): `runtime-boundaries.md` has a "Why two MCP clients and two provider builders (intentional)" section (`:138`) stating that System A uses `thinclaw_desktop_tools::McpClient` + `rig_lib::UnifiedProvider` while System B uses `thinclaw_core`'s MCP runtime + provider stack, and that the duplication is an intended consequence of the two-system design, not drift. **Documentation gap closed.**

Child-session registry cleanup — **FIXED (eviction wired, comment now accurate):**
- `sub_agent_registry::remove_parent` and `clear` now have production callers: `remove_parent` is called from `thinclaw_delete_session` (`thinclaw/commands/sessions.rs:406,432`) and `clear` from `ThinClawRuntimeState::stop` (`runtime_bridge.rs:461`), so the registry empties on session-delete and engine-stop. The `#[allow(dead_code)]` is gone and the module doc comment matches behavior. A regression test (`remove_parent_evicts_children`) covers it.

sandbox_factory tool-parse TODOs — **FIXED:**
- The `tools_used` TODO is resolved: `detect_tools_used(script)` (`rig_lib/sandbox_factory.rs:56`) best-effort parses builtin fn names and feeds `tools_used: detect_tools_used(script)` at `:122`, with unit tests at `:685-717`. No `// TODO: Parse logic` remains.

## Decision Points

1. **Cloud sync: WIRE end-to-end vs FEATURE-GATE.**
   - *Option A (build the vision):* On `migrate_to_cloud` success, call `FileStore::set_mode(Cloud)` + `set_upload_channel`, spawn an upload worker draining `UploadJob` (encrypt + `provider.put`/`delete`), spawn `SyncEngine::run` gated by `network::recommend_strategy` + `AppNapGuard`, implement the read-path cloud download in `FileStore::read`/`ensure_local`, and restore all of it from `init_from_db` on startup. Cost: real concurrency wiring + a CloudManager accessor for `provider`/`master_key`; risk of background-task lifecycle bugs.
   - *Option B (feature-gate):* Put cloud mode behind a `cloud-sync` cargo feature (default off), hide the migrate-to-cloud UI command when disabled, and leave the inert code compiling-but-unreachable. Cost: low effort but ships a visibly broken "Cloud mode" toggle or removes a marquee capability.
   - **Recommendation: Option A (build).** Everything below the orchestration layer is already written and tested; the missing piece is ~150 lines of spawn/restore glue plus the read-path. This is the workstream's headline value and squarely matches the "realize the vision" directive. Gate the *spawn* behind a runtime guard (only when `StorageMode::Cloud`) rather than a compile feature, so the capability ships.

2. **InferenceRouter chat modality: WIRE `chat.rs` onto the router vs ERASE the chat path.**
   - *Option A (wire):* Make `chat.rs::direct_chat_stream` consume `InferenceRouter::chat_backend()` instead of re-resolving the provider, and make local chat lazily `set_chat_backend(LocalChatBackend)` when the sidecar is ready. Unifies the duplicate `provider_catalog` lookup. Cost: non-trivial refactor of the hot Direct chat path (streaming, token usage, context updates) with real regression risk; `LocalChatBackend::stream_chat` maps fewer event variants than `chat.rs` currently handles.
   - *Option B (erase):* Delete `chat_backend`/`set_chat_backend`/`clear_backend(Chat)` from `InferenceRouter`, drop the chat construction from `reconfigure()`, and delete `inference/chat/{local,cloud}.rs` + the `ChatBackend` trait. `chat.rs::resolve_provider` remains the single chat path.
   - **Recommendation: Option B (erase) — this is genuinely drifted duplicate cruft.** The router's chat modality is dead, the *real* chat path (`resolve_provider`) is mature and shipping, and `runtime-boundaries.md` already names `chat.rs`/`rig_lib` as System A's owner. Re-routing the hot path through a second abstraction adds risk for zero user-visible gain. Keep the router for tts/stt/diffusion/embedding (all live). This is the rare "ERASE" call in this WS; flag it for operator sign-off because it deletes a built `ChatBackend` abstraction.

3. **Dual agent stacks: CONSOLIDATE vs DOCUMENT.**
   - *Option A (consolidate):* Merge System A's `UnifiedProvider`/`McpClient` onto System B's `thinclaw_core` runtime. Very large, cross-crate, and explicitly warned against by `runtime-boundaries.md` ("must not be collapsed into one architecture without an explicit migration plan").
   - *Option B (document):* The split is already specified; add a short addendum naming the two-MCP-client / two-provider-builder consequence as intended, so it stops reading as drift.
   - **Recommendation: Option B (document).** The dual stack is a deliberate product boundary (Direct Workbench vs Agent Cockpit), not accidental duplication. Consolidation is a separate, large initiative out of scope here. Documenting the consequence is the correct, cheap resolution.

## Tasks (12)

- [x] **T1: Add CloudManager sync accessors + `start_sync` / `stop_sync` lifecycle.**
  - **Files:** `apps/desktop/backend/src/cloud/mod.rs`.
  - **Change:** Add `pub(crate) async fn active_provider(&self) -> Option<Arc<dyn CloudProvider>>` and `pub(crate) async fn master_key(&self) -> Option<MasterKey>` reading `self.inner`. Add a `sync_handle: Option<tokio::task::JoinHandle<()>>` + the `SyncEngine`'s cancel handle (or store the `SyncEngine`) to `CloudManagerInner`, and a `pub async fn stop_sync(&self)` that cancels the engine + aborts the worker. Do **not** widen `inner` to `pub`. Keep the new fields `Option` so local mode is unaffected.
  - **Acceptance:** `CloudManager` compiles with new accessors; `cargo test -p thinclaw-desktop` cloud tests still pass; no public API on `CloudManagerInner` widened beyond `pub(crate)` accessors.
  - **Effort:** M
  - **Verification:** `cargo build -p thinclaw-desktop` (from `apps/desktop/backend`).

- [x] **T2: Implement the upload worker + sync activation in a new `cloud/live_sync.rs` submodule.**
  - **Files:** new `apps/desktop/backend/src/cloud/live_sync.rs`; register `mod live_sync;` in `cloud/mod.rs:41-52` façade.
  - **Change:** `pub(crate) async fn start_live_sync(file_store: Arc<FileStore>, cloud: Arc<CloudManager>, app_data_dir: PathBuf)` that: (a) creates `mpsc::channel::<UploadJob>(cap)`, calls `file_store.set_mode(FileStoreMode::Cloud)` + `set_upload_channel(tx)`; (b) spawns an **upload worker** that drains `rx`, holds an `AppNapGuard::begin("cloud upload")` while a batch is in flight, consults `network::recommend_strategy(&network::detect_quality(None).await).should_sync(job.data.len() as u64)` to defer large/metered uploads (re-queue or drop-with-warn per `SyncStrategy`), and for `UploadOp::Put` does `encryption::encrypt(&key, &job.rel_path, &job.data)` → `provider.put(&format!("{}.enc", job.rel_path), &enc)`, for `Delete` does `provider.delete(&format!("{}.enc", job.rel_path))`; (c) spawns `SyncEngine::default_interval().run(&mut tracker, &app_data_dir, &["documents","images","generated","vectors","previews","thinclaw"], on_changes)` where `on_changes` pushes each `ChangedFile` through the same encrypt/put path under an `AppNapGuard`. Build the initial `FileTracker` from the cloud manifest's `original_path → sha256` (download+decrypt `manifest.json.enc`, `FileTracker::load_from_hashes`). Reuse the exact key/encrypt convention from `cloud/migration.rs:280-287,676`.
  - **Acceptance:** New module isolated to one responsibility (live sync), façade `mod.rs` only declares it; worker honors `SyncStrategy`; `AppNapGuard` wraps in-flight batches; uploads use `{rel_path}.enc` + AAD = `rel_path` identical to migration.
  - **Effort:** L
  - **Verification:** `cargo build -p thinclaw-desktop`; add a unit test that drains a fake `UploadJob` through a mock `CloudProvider` (mirror `cloud/integration_tests.rs` provider fakes) asserting the `.enc` key + decrypt round-trip.

- [x] **T3: Activate live sync on `migrate_to_cloud` success.**
  - **Files:** `apps/desktop/backend/src/cloud/mod.rs:434-454` (success block), `cloud/commands.rs:329-336` (`cloud_migrate_to_cloud` already holds `app`).
  - **Change:** After setting `inner.mode = StorageMode::Cloud{..}`, call `cloud::live_sync::start_live_sync(...)`. The `FileStore` is Tauri-managed (`lib.rs:494-495`), so pass it via `app.state::<FileStore>()` inside the command (`cloud_migrate_to_cloud`) rather than deep inside `migrate_to_cloud` (keep `CloudManager` free of `AppHandle` state lookups where possible — follow the existing pattern where the command layer owns `State` access, `commands.rs:441-467`). Store the worker/engine handles in `CloudManagerInner` (T1) for shutdown.
  - **Acceptance:** After a successful migrate-to-cloud, `FileStore::mode()` returns `Cloud`, a write to `documents/x.txt` produces a `provider.put("documents/x.txt.enc", ..)`; `migrate_to_local` calls `stop_sync` + `FileStore::set_mode(Local)`.
  - **Effort:** M
  - **Verification:** `cargo build -p thinclaw-desktop`; manual smoke against MinIO (`cloud/providers/s3.rs:44` recognizes `127.0.0.1` local) per `apps/desktop/documentation/manual-smoke-checklist.md`.

- [x] **T4: Implement the read-path cloud download fallback in `FileStore`.**
  - **Files:** `apps/desktop/backend/src/file_store.rs:197-222` (`read`), `:242-262` (`ensure_local`).
  - **Change:** Give `FileStore` an optional `download: Option<Arc<dyn CloudDownloader>>` where `CloudDownloader::download(rel_path) -> Result<Vec<u8>>` is a small trait implemented in `cloud/live_sync.rs` over `provider.get("{rel}.enc")` + `encryption::decrypt(&key, rel, &enc)`. In cloud mode, when the local file is missing, download → write to local cache (`tokio::fs::write`) → return bytes, instead of erroring at `file_store.rs:215-218` / `:255-258`. Set the downloader in `start_live_sync` (T2). Keep the trait in `cloud/` so `file_store.rs` does not depend on `opendal`/`encryption` types directly (define the trait in `file_store.rs`, impl in `cloud`).
  - **Acceptance:** In cloud mode, reading a file present in cloud but absent locally returns the decrypted bytes and populates the local cache; local mode unchanged (still `NotFound`).
  - **Effort:** M
  - **Verification:** `cargo build -p thinclaw-desktop`; unit test with a mock downloader asserting cache-fill + bytes returned.

- [x] **T5: Restore live sync on startup when already in cloud mode.**
  - **Files:** `apps/desktop/backend/src/cloud/mod.rs:147-258` (`init_from_db`) or `lib.rs:483-495` (preferred — keep `AppHandle`/`FileStore` state access in `lib.rs`).
  - **Change:** After `init_from_db` succeeds and mode is `StorageMode::Cloud`, and provider+master_key are present, call `cloud::live_sync::start_live_sync(...)` from `lib.rs` (where `FileStore` is in scope just below). On `RunEvent::Exit` (`lib.rs:645-652`), call `cloud.stop_sync().await` before shutdown so the worker/engine drain cleanly.
  - **Acceptance:** Launching the app with persisted cloud mode spawns the worker + engine without a manual migration; quitting stops them.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw-desktop`; smoke: set cloud mode, restart app, confirm `[file_store] Mode changed to: Cloud` + `[cloud/sync] Starting sync loop` in logs.

- [x] **T6: Fix `last_modified = 0` in all three opendal providers (fix the whole class, not one copy).**
  - **Files:** `apps/desktop/backend/src/cloud/providers/s3.rs:142`, `webdav.rs:136`, `sftp.rs:131`.
  - **Change:** Replace the hardcoded `last_modified: 0` with extraction from opendal `Metadata`: `meta.last_modified().map(|t| t.timestamp_millis()).unwrap_or(0)` (opendal 0.55 returns `Option<chrono::DateTime<Utc>>`). Apply identically to all three call sites. Use millis to match the native providers (`icloud.rs:379` uses `as_millis`).
  - **Acceptance:** All three providers populate `CloudEntry.last_modified` from real metadata; `grep -rn "last_modified: 0" src/cloud/providers` returns nothing except `integration_tests.rs` fixtures.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw-desktop`; `cargo clippy -p thinclaw-desktop --all-targets -- -D warnings`.

- [x] **T7: ERASE the dead InferenceRouter chat modality (pending Decision #2 sign-off).**
  - **Files:** `apps/desktop/backend/src/inference/router.rs` (remove `chat` field at `:45`, `chat_backend()` `:87`, `set_chat_backend()` `:121`, `clear_backend(Chat)` arm `:148`, chat construction in `reconfigure()` `:279-318`, and the `Modality::Chat` arm in `active_backends()`/`available_backends_for()`); `inference/chat/local.rs`, `inference/chat/cloud.rs`, `inference/chat/mod.rs` (delete trait + impls); `inference/mod.rs` (remove `Modality::Chat` handling at `:274` and the `pub mod chat`); check `inference/model_discovery` for `Modality::Chat` references.
  - **Change:** Remove the dead chat path. Keep `chat.rs::resolve_provider` as the sole chat path. Leave tts/stt/diffusion/embedding modalities fully intact. If `Modality::Chat` is load-bearing elsewhere (e.g. `available_backends_for` is surfaced in a settings UI), retain the *enum variant* and keep `available_backends_for(Chat)` (it reads config, not the router field) but still drop the unread `Arc<dyn ChatBackend>` storage.
  - **Acceptance:** `grep -rn "ChatBackend\|set_chat_backend\|chat_backend()" apps/desktop/backend/src/inference` returns only intentional remnants; chat still streams via `direct_chat_stream`.
  - **Effort:** M
  - **Verification:** `cargo build -p thinclaw-desktop`; `cargo clippy -p thinclaw-desktop --all-targets -- -D warnings` (catches any dangling reference); smoke a Direct chat turn.

- [x] **T8: Wire `sub_agent_registry::remove_parent` to session-delete and `clear` to engine-stop.**
  - **Files:** `apps/desktop/backend/src/thinclaw/commands/sessions.rs:323-337` (local delete branch — also handle the remote branch at `:317-321` if children can be tracked remotely; if not, only local), `apps/desktop/backend/src/thinclaw/runtime_bridge.rs:454-457` (`stop()`, next to the existing `active_sessions.write().await.clear()`).
  - **Change:** In `thinclaw_delete_session`, after a successful delete, call `crate::thinclaw::commands::rpc_orchestration::sub_agent_registry::remove_parent(&session_key).await` (make it `pub(crate)` if needed; drop `#[allow(dead_code)]`). In `ThinClawRuntimeState::stop`, call `...::sub_agent_registry::clear().await`. Update the module doc comment (`rpc_orchestration.rs:21-22`) to remain accurate.
  - **Acceptance:** Both functions have callers; `#[allow(dead_code)]` removed from `rpc_orchestration.rs:103,110`; deleting a parent session removes its children; stopping the engine empties the registry.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw-desktop`; `cargo clippy -p thinclaw-desktop --all-targets -- -D warnings` (would flag dead code if still unused); unit test registering children then asserting `all_children() == 0` after `clear()`.

- [x] **T9: Resolve the `sandbox_factory` tool-parse TODOs.**
  - **Files:** `apps/desktop/backend/src/rig_lib/sandbox_factory.rs:304-305`.
  - **Change:** Populate `SkillManifest.tools_used` by parsing the Rhai script for the registered builtin fn names the sandbox exposes (e.g. scan `script` for `mcp_call`/`web_search`/known builtin idents registered in this same function), and `parameters` from a documented convention (or, if no convention exists yet, replace the TODO with an explicit comment that params are not yet a supported skill-save input and remove the misleading TODO). Prefer the minimal honest fix over inventing a parameter schema.
  - **Acceptance:** No `// TODO` remains at those lines; `tools_used` is best-effort populated OR the comment states the deliberate limitation; behavior unchanged for skill save.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw-desktop`; save a skill from the sandbox and confirm the manifest `tools_used` reflects detected builtins (or the limitation is documented).

- [x] **T10: Document the intentional dual-stack two-MCP-client / two-provider-builder consequence.**
  - **Files:** `apps/desktop/documentation/runtime-boundaries.md` (extend the "Shared Infrastructure" or add a short "Why two MCP clients / provider builders" note near the System A/B definitions).
  - **Change:** Add a paragraph stating that System A uses `thinclaw_desktop_tools::McpClient` + `rig_lib::UnifiedProvider` while System B uses `thinclaw_core`'s MCP runtime + provider stack, that this duplication is intentional (the two systems must not share tool authority or provider routing), and that the duplication is not to be "fixed" by collapsing them without the migration plan the doc already requires. Cross-reference the audit finding so future maintainers don't re-file it as drift.
  - **Acceptance:** `runtime-boundaries.md` explicitly names the two MCP clients and two provider builders as intended; "Last updated" date bumped.
  - **Effort:** S
  - **Verification:** Doc review; `markdownlint` if configured; no code change.

- [x] **T11: Guard the image-gen progress divide-by-zero `%` label.**
  - **Files:** `apps/desktop/backend/src/image_gen.rs:700`.
  - **Change:** Guard the denominator before computing the progress percentage so a zero total no longer yields a garbage (`NaN`/`inf`) `%` label. Clamp to `0`/`100` (or show an indeterminate label) when the total is zero. Display-only fix — this never crashes today, it just renders a bad string.
  - **Acceptance:** With a zero total, the progress label shows a sane value (e.g. `0%`) instead of `NaN%`/`inf%`; non-zero totals are unchanged.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw-desktop`; `cargo clippy -p thinclaw-desktop --all-targets -- -D warnings`; unit-check the percentage helper with a zero denominator.

- [ ] **T12: Resolve the desktop build profile omitting the `wasm-runtime` feature. OPEN.** The `desktop` profile (`Cargo.toml`) still reads `desktop = ["libsql", "html-to-markdown", "document-extraction", "repl", "timezones"]` with only a general `# minimal footprint` comment. `wasm-runtime` was neither added nor given an explicit inline rationale for its omission, so this task did not land.
  - **Files:** root `Cargo.toml` `[features] desktop = [...]`.
  - **Change:** Either add `wasm-runtime` to the desktop profile feature list, or explicitly document why it is omitted with an inline comment in the `[features]` block. Coordinate the doc note with **WS-12** so the rationale stays consistent with the WASM-runtime feature-flag ownership there.
  - **Acceptance:** The desktop profile either pulls in `wasm-runtime` or carries a clear comment explaining the deliberate omission; the choice is reconciled with WS-12's WASM feature-flag documentation.
  - **Effort:** S
  - **Verification:** `cargo build` of the desktop profile reflects the chosen feature set; doc review with WS-12 if the omission is documented rather than fixed.

## Best Practices (workstream-specific)

- **Reuse the migration encrypt/key convention verbatim.** The live upload worker (T2) and read-path (T4) must use `encryption::encrypt(master_key, relative_path, data)` with cloud key `format!("{}.enc", relative_path)` exactly as `cloud/migration.rs:280-287,676` does — the AAD is the `relative_path`, so any divergence makes uploaded files undecryptable on restore. Cite `migration.rs` as the canonical pattern.
- **Keep `CloudManagerInner` private.** Add `pub(crate)` accessors, not public fields (matches the existing `provider_config()` accessor at `cloud/mod.rs:327`). The lock discipline (`RwLock<CloudManagerInner>`) is load-bearing for the migration mutex.
- **Honor `SyncStrategy` in the worker.** `network::recommend_strategy` + `SyncStrategy::should_sync(size)` already encode metered/offline/cellular/large-file policy (`cloud/network.rs:79-96,164-192`) — call them; do not reinvent thresholds.
- **Wrap long uploads in `AppNapGuard`** (`cloud/app_nap.rs:74`) the way the doc comment intends — it is ref-counted, so per-batch guards compose safely.
- **Follow façade-module hygiene (CLAUDE.md).** Put live-sync glue in a new `cloud/live_sync.rs`, declared in the `cloud/mod.rs` façade (`:41-52`); do not grow `cloud/mod.rs` (727L) or `lib.rs` with worker bodies.
- **Command layer owns `State`/`AppHandle` access.** `cloud/commands.rs` already owns `State<CloudManager>`/`State<SqlitePool>`/`AppHandle` (`commands.rs:329-336,441-467`); thread `FileStore` access through the command, keeping `CloudManager` methods state-lookup-free where practical.
- **Provider-class fixes must touch every copy.** When fixing opendal-derived fields (T6), grep the whole `cloud/providers/` dir — the `last_modified` and any future `Metadata` field lives in 3 opendal providers; the gdrive/onedrive/dropbox/icloud ones use native APIs and differ.

## Common Pitfalls

- **Fixing `last_modified` in only S3.** The audit anchor names `s3.rs:142`, but `webdav.rs:136` and `sftp.rs:131` carry the identical bug. This is the exact "split_message landed in only one of four copies" trap the audit calls out (AUDIT-FINDINGS §5). T6 must hit all three.
- **Re-routing the Direct chat hot path through the router.** `LocalChatBackend::stream_chat` maps fewer `ProviderEvent` variants than `chat.rs` handles today; "wiring" the router instead of erasing it (Decision #2) risks dropping token-usage / context-update events on the most-used surface for no user gain.
- **Encrypting with the wrong AAD.** If the worker passes the cloud key (`x.enc`) instead of the relative path (`x`) as the second arg to `encryption::encrypt`, files upload but fail to decrypt on a new device — silently, until restore. Match `migration.rs` exactly.
- **Leaking the upload worker on mode flip / shutdown.** Spawning the worker + `SyncEngine` without storing their handles means `migrate_to_local`, a second `migrate_to_cloud`, or app exit can leave orphaned background tasks racing the next worker. Store handles in `CloudManagerInner` (T1) and cancel in `stop_sync` (T3/T5).
- **`try_send` on a full bounded channel drops writes.** `FileStore` uses `try_send` and only warns on a full queue (`file_store.rs:159-164`). Size the channel generously and ensure the worker keeps draining; a slow provider must back-pressure via the `SyncEngine` cadence, not by silently losing `UploadJob`s.
- **Editing WS-10's god-files.** Do not refactor `rpc_dashboard.rs`/`remote_proxy.rs`/`sidecar.rs` here beyond the one-line registry calls in `sessions.rs`/`runtime_bridge.rs` (T8). Larger decomposition is WS-10's.
- **Treating the dual stack as drift.** Do not "consolidate" the two MCP clients/providers — `runtime-boundaries.md` forbids collapsing the systems without a migration plan. T10 documents, it does not merge.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Sequential spine (one worker):** T1 → T2 → T3 → T4 → T5. These all touch `cloud/mod.rs` / `live_sync.rs` / `file_store.rs` and have a hard dependency chain (accessors → worker → activation → read-path → startup). Keep them in one worker to avoid merge churn.
  - **Parallel fan-out (independent files):** T6 (`cloud/providers/{s3,webdav,sftp}.rs`), T7 (`inference/**` — gated on Decision #2), T8 (`sessions.rs` + `runtime_bridge.rs` + `rpc_orchestration.rs`), T9 (`sandbox_factory.rs`), T10 (`runtime-boundaries.md`), T11 (`image_gen.rs`), T12 (root `Cargo.toml` `[features]` — coordinate with WS-12) touch disjoint files and can run as separate concurrent workers.
- **Isolation:** The cloud spine (T1-T5) and each parallel task touch disjoint files, so **git worktree isolation is recommended** for the fan-out workers to allow true parallel mutation, with a final integration merge. The spine worker is the long pole; the five fan-out tasks can land independently behind it.
- **Workflow shape:** implement → verify → review → fix.
  - *Implement:* spine worker runs T1-T5 sequentially; fan-out workers (T6,T7,T8,T9,T10) run concurrently in separate worktrees.
  - *Verify:* each worker runs the gate below in its worktree.
  - *Review:* `/code-review` on the combined diff (high effort — touches a security-sensitive crypto upload path; T2/T4 AAD correctness deserves scrutiny). `/security-review` specifically on T2/T4 (encryption key handling, AAD, cache-fill).
  - *Fix:* address review findings, re-run gate.
- **Verification gate (run from `apps/desktop/backend`, the excluded package — NOT the workspace root):**
  - `cargo fmt`
  - `cargo clippy -p thinclaw-desktop --all-targets --all-features -- -D warnings`
  - `cargo test -p thinclaw-desktop` (cloud unit tests in `cloud/sync.rs`, `cloud/network.rs`, `cloud/app_nap.rs`, `cloud/integration_tests.rs`, plus new T2/T4/T8 tests)
  - `/ship` for the consolidated Rust quality gate.
  - **DB/Docker prerequisites:** T3 manual smoke needs a local S3-compatible endpoint — `docker run -p 9000:9000 minio/minio server /data` (the S3 provider recognizes `127.0.0.1`/`localhost`, `s3.rs:44`). No Postgres needed (desktop uses SQLite + libSQL runtime). If Docker is unhealthy, follow the disk-pressure recovery in CLAUDE.md before assuming a product bug.

## Definition of Done

- [x] Decision #1 (build) resolved: live sync spawns on migrate-to-cloud and on startup; reads fall back to cloud download; quitting stops the worker + engine.
- [x] Decision #2 resolved: dead `InferenceRouter` chat modality erased — chat still streams via `direct_chat_stream`.
- [x] Decision #3 resolved: `runtime-boundaries.md` documents the intentional two-MCP-client / two-provider-builder split.
- [x] `last_modified` populated in all three opendal providers; `grep "last_modified: 0" src/cloud/providers` clean (excluding test fixtures).
- [x] `sub_agent_registry::remove_parent`/`clear` have real callers; `#[allow(dead_code)]` removed; module doc comment accurate; registry empties on session-delete + engine-stop.
- [x] `sandbox_factory.rs` TODOs resolved (`detect_tools_used` populates `tools_used`).
- [x] `image_gen.rs` progress `%` label guards a zero denominator via `progress_fraction` (sane value instead of `NaN%`/`inf%`).
- [ ] Desktop build profile's `wasm-runtime` omission resolved (added to `[features] desktop = [...]` or documented, coordinated with WS-12). **Open (T12):** `desktop` still omits `wasm-runtime` and carries only a general "minimal footprint" comment, not an explicit rationale for the omission.
- [x] `cargo fmt` clean, `cargo clippy -p thinclaw-desktop --all-targets -- -D warnings` green, `cargo test -p thinclaw-desktop` green (incl. new tests).
- [x] `/code-review` and `/security-review` of T2/T4 pass with no unresolved correctness/crypto findings.
- [x] No files owned by other workstreams (WS-10 god-files, root crates) modified beyond the documented one-line registry hooks.
