# Scrappy — Implementation TODO

> **Date:** 2026-03-01  
> **Scope:** Three work streams: (C) Wiring remaining cloud backends into live command paths, (D) Cloud Model Discovery, (A) Cloud Storage.  
> **Prerequisite:** IronClaw integration ✅, InferenceRouter ✅, cloud backend implementations ✅

---

## Table of Contents

1. [Current State: What's Already Built](#current-state-whats-already-built)
2. [Work Stream C — Wire Cloud Backends Into Live Paths](#work-stream-c--wire-cloud-backends-into-live-paths)
   - C1. [TTS: Route Through InferenceRouter](#c1-tts-route-through-inferencerouter)
   - C2. [STT: Route Through InferenceRouter](#c2-stt-route-through-inferencerouter)
   - C3. [Diffusion: Route Through InferenceRouter](#c3-diffusion-route-through-inferencerouter)
   - C4. [Frontend Badges & Controls](#c4-frontend-badges--controls)
   - C5. [Specta Bindings Regen](#c5-specta-bindings-regen)
3. [Work Stream D — Cloud Model Discovery](#work-stream-d--cloud-model-discovery)
   - D1. [CloudModelRegistry (Backend)](#d1-cloudmodelregistry-backend)
   - D2. [Provider Discovery Modules](#d2-provider-discovery-modules)
   - D3. [Frontend Integration](#d3-frontend-integration)
   - D4. [Context Size Propagation](#d4-context-size-propagation)
4. [Work Stream A — Cloud Storage](#work-stream-a--cloud-storage)
   - A1. [Foundation](#a1-foundation)
   - A2. [Migration Engine](#a2-migration-engine)
   - A3. [FileStore Abstraction](#a3-filestore-abstraction)
   - A4. [Additional Cloud Providers](#a4-additional-cloud-providers)
   - A5. [Frontend UI](#a5-frontend-ui)
   - A6. [Tests & Polish](#a6-tests--polish)
5. [Post-Integration Enhancements](#post-integration-enhancements--complete)
   - P1. [Session Context Routing](#p1-session-context-routing-)
   - P2. [Hot Config Reload](#p2-hot-config-reload-)
   - P3. [Sub-Agent Spawning](#p3-sub-agent-spawning-)
6. [Dependency Map](#dependency-map)
7. [Open Decisions](#open-decisions)

---

## Current State: What's Already Built

### InferenceRouter (fully wired)

```
backend/src/inference/
├── mod.rs                  ← InferenceRouter struct, Tauri commands, shared types
├── router.rs               ← reconfigure() reads UserConfig + SecretStore → activates backends
├── provider_endpoints.rs   ← 14 cloud chat providers (base_url, api_compat, default_model)
├── chat/
│   ├── mod.rs              ← ChatBackend trait
│   ├── local.rs            ← wraps EngineManager
│   └── cloud.rs            ← wraps UnifiedProvider (14 providers)
├── embedding/
│   ├── mod.rs              ← EmbeddingBackend trait (dimensions(), embed_batch())
│   ├── local.rs            ← wraps llama-server / mlx-embed
│   ├── cloud_openai.rs     ← ✅ text-embedding-3-small/large (OpenAI)
│   ├── cloud_gemini.rs     ← ✅ text-embedding-004 (Gemini)
│   ├── cloud_voyage.rs     ← ✅ voyage-3 / voyage-3-large (Voyage AI)
│   └── cloud_cohere.rs     ← ✅ embed-v3 (Cohere)
├── tts/
│   ├── mod.rs              ← TtsBackend trait (synthesize(), available_voices())
│   ├── local.rs            ← stub (Piper runs via tts.rs directly)
│   ├── cloud_openai.rs     ← ✅ IMPLEMENTED (tts-1, tts-1-hd, 6 voices, MP3 output)
│   ├── cloud_elevenlabs.rs ← ✅ IMPLEMENTED
│   └── cloud_gemini.rs     ← ✅ IMPLEMENTED
├── stt/
│   ├── mod.rs              ← SttBackend trait (transcribe())
│   ├── local.rs            ← stub (whisper runs via stt.rs directly)
│   ├── cloud_openai.rs     ← ✅ IMPLEMENTED (whisper-1, multipart upload)
│   ├── cloud_gemini.rs     ← ✅ IMPLEMENTED
│   └── cloud_deepgram.rs   ← ✅ IMPLEMENTED
└── diffusion/
    ├── mod.rs              ← DiffusionBackend trait (generate())
    ├── local.rs            ← stub (sd.cpp runs via image_gen.rs directly)
    ├── cloud_imagen.rs     ← ✅ IMPLEMENTED (Imagen 3 Flash/Pro)
    ├── cloud_dalle.rs      ← ✅ IMPLEMENTED (DALL-E 3)
    ├── cloud_stability.rs  ← ✅ IMPLEMENTED (SDXL)
    ├── cloud_fal.rs        ← ✅ IMPLEMENTED (FLUX dev/schnell)
    └── cloud_together.rs   ← ✅ IMPLEMENTED (Together AI)
```

### The Gap — ✅ CLOSED (2026-03-01)

~~All 12 cloud backends are **fully implemented** with `reqwest` HTTP calls, error handling, and auth.
But the **live Tauri commands** (`tts_synthesize`, `transcribe_audio`, `imagine_generate`) still call the **old code paths** directly.~~

**RESOLVED:** All three live commands now route through `InferenceRouter`. The `update_inference_backend` command also now calls `router.reconfigure()` to activate backends immediately (previously it just logged intent).

---

## Work Stream C — Wire Cloud Backends Into Live Paths  ✅ COMPLETE

> **Goal:** Make `tts_synthesize`, `transcribe_audio`, and `imagine_generate` use `InferenceRouter` so users can switch between local and cloud backends in Settings.
>
> **Completed:** 2026-03-01 (~30 minutes)

### C1. TTS: Route Through InferenceRouter ✅

**Completed 2026-03-01.** Added `State<'_, InferenceRouter>` to `tts_synthesize`. Cloud branch inserted before Piper spawn — calls `backend.synthesize()` and returns base64. `decodeAudioData()` handles MP3/PCM transparently.

---

### C2. STT: Route Through InferenceRouter ✅

**Completed 2026-03-01.** Added `State<'_, InferenceRouter>` to `transcribe_audio`. Cloud branch inserted before whisper-server check — calls `backend.transcribe()` for OpenAI Whisper, Deepgram, or Gemini STT. Local fallback preserved.

### C3. Diffusion: Route Through InferenceRouter ✅

**Completed 2026-03-01.** Major refactor of `imagine.rs`:
- **Deleted** ~190-line `generate_with_gemini()` duplicate (now handled by `ImagenDiffusionBackend`)
- **Added** `generate_with_cloud_backend()` helper → routes through InferenceRouter
- All cloud providers (`nano-banana`, `nano-banana-pro`, `gemini`, `openai`, `stability`, `fal`, `together`) route through router
- Local sd.cpp/mflux path preserved as fallback
- Cleaned up unused imports (`base64::prelude::*`, `uuid::Uuid`, `Manager`)

**Also fixed: `update_inference_backend`** — was only logging intent, now calls `router.reconfigure()` to activate backends immediately.

---

### C4. Frontend Badges & Controls ✅

| # | Task | File | Status |
|---|------|------|--------|
| C4-1 | STT badge on microphone button | `ChatInput.tsx` | ✅ Shows 🖥️ Local / ☁️ Provider below mic icon via `useInferenceBackends` hook. |
| C4-2 | TTS badge on "Read Aloud" button | `MessageBubble.tsx` | ✅ Shows provider name on hover via `useInferenceBackends` hook. |
| C4-3 | TTS voice selector (ElevenLabs/OpenAI/Gemini) | `InferenceModeTab.tsx`, `tts.rs` | ✅ Collapsible voice picker panel + `tts_list_voices` Tauri command + voice persisted in config. |
| C4-4 | Cost estimation for cloud backends | `InferenceModeTab.tsx` | ✅ Shows $/1M tokens, $/image, $/min, $/1K chars from discovery pricing data. |

---

### C5. Specta Bindings Regen ✅

**Completed.** `cargo tauri dev` regenerated `frontend/src/lib/bindings.ts`. Verified: `ttsListVoices`, `VoiceInfo`, `selected_model_context_size`, `discoverCloudModels`, `refreshCloudModels`, `openclawReloadSecrets` all present.

---

## Post-Integration Enhancements ✅ COMPLETE

> **Goal:** Harden the IronClaw integration with session-safe event routing, hot API key reloading, and sub-agent spawning.
>
> **Completed:** 2026-03-01

### P1. Session Context Routing ✅

**Problem:** `TauriChannel::send_status()` relied on `metadata.get("thread_id")` which may not always be present, causing StatusUpdate events to route to the wrong session in multi-session scenarios.

**Solution:** Added `session_context: Arc<RwLock<String>>` to `TauriChannel`. Tauri commands set this before calling `send_message()` / `resolve_approval()`, providing a reliable fallback when IronClaw's metadata doesn't include `thread_id`.

| File | Change |
|------|--------|
| `ironclaw_channel.rs` | Added `session_context` field, 3-tuple `new()` return, fallback in `send_status()` |
| `ironclaw_bridge.rs` | Store `session_context` in `IronClawInner`, expose `set_session_context()` |
| `sessions.rs` | Call `set_session_context()` before `send_message()` and `resolve_approval()` |
| `rpc.rs` | Call `set_session_context()` before `openclaw_spawn_session()` |

---

### P2. Hot Config Reload ✅

**Problem:** When users save/toggle API keys in Settings, changes persisted to Keychain + identity.json but the running IronClaw agent didn't pick them up until restart.

**Solution:** Added `openclaw_reload_secrets` Tauri command that performs a graceful stop→start cycle. The `KeychainSecretsAdapter` reads directly from macOS Keychain, so restarting naturally re-injects all keys via `inject_llm_keys_from_secrets()`. Frontend `SecretCard.handleSave()` fires this in background after successful saves.

| File | Change |
|------|--------|
| `ironclaw_bridge.rs` | Added `reload_secrets(secrets_store)` method (stop→start cycle) |
| `gateway.rs` | Added `openclaw_reload_secrets` Tauri command |
| `lib.rs` | Registered `openclaw_reload_secrets` in invoke handler |
| `openclaw.ts` | Added `reloadSecrets()` frontend API wrapper |
| `SecretsTab.tsx` | Fire-and-forget `reloadSecrets()` after successful key save |

---

### P3. Sub-Agent Spawning ✅

**Problem:** Users could only interact with `agent:main`. No way to spawn parallel task sessions.

**Solution:** `openclaw_spawn_session` already existed and was correctly implemented — creates a UUID-based session key and sends the task as first message via `ironclaw::api::chat::send_message()`. Enhanced with session context routing so spawned session events route correctly.

| File | Change |
|------|--------|
| `rpc.rs` | Added `set_session_context()` call before spawn |

---

## Work Stream D — Cloud Model Discovery

> **Goal:** Replace hardcoded `model-library.ts` cloud entries with live API queries to each provider.
>
> **Estimated effort:** 3–5 days

### D1. CloudModelRegistry (Backend) ✅

**Completed 2026-03-01.** Created `inference/model_discovery/` module:
- `mod.rs` — `CloudModelRegistry` with `RwLock<HashMap>` cache (30-min TTL), parallel discovery, and `SecretStore` integration
- `types.rs` — `CloudModelEntry`, `ModelCategory`, `ModelPricing`, `DiscoveryResult`, `ProviderDiscoveryResult` (all Specta-derived)
- `classifier.rs` — Pattern-matching rules per provider
- Registered as Tauri managed state in `lib.rs`
- Added `discover_cloud_models` and `refresh_cloud_models` Tauri commands

### D2. Provider Discovery Modules ✅

**Completed 2026-03-01.** All 12 provider modules + 1 static registry implemented:

| Provider | File | Strategy |
|----------|------|----------|
| ✅ OpenAI | `openai.rs` | Pattern matching + known limits/pricing |
| ✅ Anthropic | `anthropic.rs` | All Chat + known pricing |
| ✅ Gemini | `gemini.rs` | `supportedGenerationMethods` classification |
| ✅ Groq | `groq.rs` | `whisper-*` → STT, else Chat |
| ✅ OpenRouter | `openrouter.rs` | All Chat + pricing extraction |
| ✅ Mistral | `mistral.rs` | `embed` → Embedding, else Chat |
| ✅ xAI | `xai.rs` | All Chat |
| ✅ Together | `together.rs` | `type` field classification |
| ✅ Cohere | `cohere.rs` | `endpoints[]` classification |
| ✅ ElevenLabs | `elevenlabs.rs` | All TTS |
| ✅ Stability | `stability.rs` | All Diffusion |
| ✅ Static | `static_registry.rs` | Deepgram (STT), Voyage (Embedding), fal.ai (Diffusion) |

**Note:** Venice not yet implemented (no known API endpoint). Can be added as static registry later.

### D3. Frontend Integration ✅

| # | Task | File | Status |
|---|------|------|--------|
| D3-1 | Create `useCloudModels` hook | `hooks/use-cloud-models.ts` | ✅ Calls `discover_cloud_models` on mount, caches in React state. Exposes `{ models, modelsByCategory, loading, refreshProvider, refreshAll }`. |
| D3-2 | Integrate into `ModelBrowser.tsx` | `ModelBrowser.tsx` | ✅ Cloud-discovered chat models merged into unified list with deduplication. Cloud loading indicator + refresh + error badges in header. |
| D3-3 | Integrate into `ModelSelector.tsx` | `ModelSelector.tsx` | ✅ Cloud-discovered chat models merged with hardcoded entries in dropdown, deduplicated, with provider badges. |
| D3-4 | Integrate into `InferenceModeTab.tsx` | `InferenceModeTab.tsx` | ✅ Each modality card shows discovered models for the active provider (up to 8 with hover tooltips for context/pricing). |
| D3-5 | Annotate `model-library.ts` | `model-library.ts` | ✅ Cloud entries marked as offline fallback. Deduplication in `useCloudModels` prevents duplicates when live discovery is active. |
| D3-6 | Error handling | `ModelBrowser.tsx` | ✅ Per-provider error badges (⚠️ N providers failed) with hover tooltip showing details. Global discovery failure shown as "Discovery failed". |

### D4. Context Size Propagation ✅

| # | Task | File | Status |
|---|------|------|--------|
| D4-1 | Add `selected_model_context_size: Option<u32>` to `UserConfig` | `config.rs` | ✅ New field with `#[serde(default)]`. Falls back to `None` (uses provider default). |
| D4-2 | Read context size in `resolve_provider()` + `CloudChatBackend` | `chat.rs`, `cloud.rs`, `router.rs` | ✅ `unwrap_or(endpoint.default_context_size)` in all 3 paths. |
| D4-3 | Frontend: write context size on model select | `ModelSelector.tsx`, `ModelBrowser.tsx` | ✅ Both selection handlers write `selected_model_context_size` from discovery metadata. |

---

## Work Stream A — Cloud Storage

> **Goal:** Enable cloud sync of all app data (SQLite, images, documents) to S3-compatible storage.
>
> **Reference design:** `documentation/latest/cloud_storage_implementation.md` (66 KB)
>
> **Estimated effort:** 3–4 weeks

### A1. Foundation ✅

**Completed 2026-03-01.** All 9 foundation tasks done. Build clean, 13 tests pass.

| # | Task | File | Status |
|---|------|------|--------|
| A1-1 | Dependencies: `opendal` 0.55, `aes-gcm`, `hkdf`, `zstd` | `Cargo.toml` | ✅ |
| A1-2 | Module tree: 8 files under `cloud/` | `cloud/` | ✅ |
| A1-3 | `CloudProvider` trait + `CloudError`, `CloudEntry`, `CloudStatus`, `CloudProviderConfig` | `cloud/provider.rs` | ✅ 175 LOC |
| A1-4 | S3 provider via opendal (AWS, R2, B2, Wasabi, DO, MinIO) | `cloud/providers/s3.rs` | ✅ 200 LOC |
| A1-5 | AES-256-GCM + HKDF-SHA256 + macOS Keychain + recovery key export/import | `cloud/encryption.rs` | ✅ 320 LOC, 9 tests |
| A1-6 | SQLite `VACUUM INTO` + WAL checkpoint snapshots | `cloud/snapshot.rs` | ✅ 95 LOC |
| A1-7 | `cloud_config` + `cloud_migrations` tables | `migrations/20260301000000_cloud_storage.sql` | ✅ |
| A1-8 | `ArchiveManifest` JSON + SHA-256 checksums + file type classification | `cloud/manifest.rs` | ✅ 230 LOC, 4 tests |
| A1-9 | `CloudManager` managed state + 8 Tauri commands registered | `cloud/commands.rs`, `lib.rs` | ✅ 237 LOC |

**Module tree:**
```
backend/src/cloud/
├── mod.rs              — CloudManager, StorageMode, init_from_db()
├── commands.rs         — 8 Tauri commands (specta-derived types)
├── provider.rs         — CloudProvider trait, CloudError, factory (7 providers)
├── providers/
│   ├── mod.rs
│   ├── s3.rs           — S3Provider via opendal
│   ├── icloud.rs       — iCloud Drive via native FS (320 LOC, 3 tests)
│   ├── gdrive.rs       — Google Drive via API v3 (500 LOC, 3 tests)
│   ├── dropbox.rs      — Dropbox via API v2 (430 LOC, 5 tests)
│   ├── onedrive.rs     — OneDrive via Graph API (440 LOC, 5 tests)
│   ├── webdav.rs       — WebDAV via opendal (230 LOC, 3 tests)
│   └── sftp.rs         — SFTP via opendal (215 LOC, 1 test)
├── oauth.rs            — OAuth 2.0 PKCE manager (450 LOC, 8 tests)
├── encryption.rs       — AES-256-GCM + HKDF + Keychain (18 tests)
├── snapshot.rs         — VACUUM INTO snapshots
├── manifest.rs         — ArchiveManifest + SHA-256 (14 tests)
├── progress.rs         — ProgressTracker + MigrationProgress events
├── migration.rs        — run_to_cloud() + run_to_local()
├── sync.rs             — SyncEngine + FileTracker (340 LOC, 7 tests)
├── app_nap.rs          — macOS App Nap RAII guard (2 tests)
├── network.rs          — Network quality detection (200 LOC, 9 tests)
└── integration_tests.rs — MockProvider + 10 integration tests
```

### A2. Migration Engine
### A2. Migration Engine ✅ (Core)

**Completed 2026-03-01.** Core migration engine implemented in `cloud/migration.rs` (683 LOC).

| # | Task | File | Status |
|---|------|------|--------|
| A2-1 | `migrate_to_cloud()`: 7-phase flow (preflight → snapshot → encrypt+upload → manifest → verify → mode switch → cleanup) | `cloud/migration.rs` | ✅ |
| A2-2 | `migrate_to_local()`: 6-phase flow (preflight → manifest → download+decrypt → restore DB → mode switch → cleanup) | `cloud/migration.rs` | ✅ |
| A2-3 | Progress reporting: `ProgressTracker` emits `cloud_migration_progress` Tauri events per file with phase, speed, ETA | `cloud/progress.rs` | ✅ |
| A2-4 | Register all cloud Tauri commands | `lib.rs` | ✅ Done in A1-9 |
| A2-5 | Migration resume on launch: `cloud_migrations.status = 'in_progress'` detection | `cloud/mod.rs` | ✅ in `init_from_db()` |
| A2-6 | Periodic background sync: `SyncEngine` + `FileTracker` (SHA-256 change detection, 5-min batch) | `cloud/sync.rs` (340 LOC, 7 tests) | ✅ |
| A2-7 | Master key in macOS Keychain | `cloud/encryption.rs` | ✅ Done in A1-5 |

**Key features:**
- Cancellable migrations with `cancel_flag` checked between files
- Spot-check verification (3 random files re-downloaded, decrypted, SHA-256 verified)
- DB progress checkpointing for resume detection
- File type-aware phase reporting (DB → Documents → Images → Vectors → Agent State)
- Failure recording in `cloud_migrations` table

### A3. FileStore Abstraction ✅

> **Completed 2026-03-01.** Centralized file I/O through `FileStore` managed state.

| # | Task | File | Status |
|---|------|------|--------|
| A3-1 | `FileStore` struct as Tauri managed state: `write()`, `read()`, `ensure_local()`, `delete()`, `exists()`, `list()`, `copy()`, `rename()`, `metadata()` | `file_store.rs` | ✅ 310 LOC |
| A3-2 | Local-only implementation. Pass-through to `tokio::fs`. Zero behavior change. | `file_store.rs`, `lib.rs` | ✅ Registered |
| A3-3 | Migrate `images.rs` — 3 call sites: `create_dir_all`, `read`, `exists` | `images.rs` | ✅ |
| A3-4 | Migrate `imagine.rs` — 1 call site: `remove_file` → async | `imagine.rs` | ✅ |
| A3-5 | Migrate `rag.rs` — 4 call sites: `create_dir_all`, `write` (doc + 2 previews) | `rag.rs` | ✅ |
| A3-6 | Migrate `projects.rs` — 1 call site: `remove_file` → async | `projects.rs` | ✅ |
| A3-7 | Migrate `history.rs` — 4 call sites: `remove_dir_all` + `create_dir_all` for docs/images | `history.rs` | ✅ |
| A3-8 | `openclaw/commands/sessions.rs` — N/A (IronClaw workspace, outside `app_data_dir`) | `sessions.rs` | ⏭️ Skipped |
| A3-9 | Cloud backend: `write()` → local + queue upload via `mpsc`; `read()` → local-first + cloud fallback; `delete()` → queue cloud deletion; `copy()`/`rename()` → queue ops | `file_store.rs` (380 LOC) | ✅ |

### A4. Additional Cloud Providers ✅ COMPLETE

| # | Task | File | Status |
|---|------|------|--------|
| A4-1 | iCloud Drive provider via native macOS filesystem + iCloud container | `cloud/providers/icloud.rs` (320 LOC, 3 tests) | ✅ |
| A4-2 | Google Drive provider via Drive API v3 + OAuth 2.0 PKCE | `cloud/providers/gdrive.rs` (500 LOC, 3 tests) | ✅ |
| A4-3 | Dropbox provider via Dropbox API v2 + OAuth 2.0 PKCE | `cloud/providers/dropbox.rs` (430 LOC, 5 tests) | ✅ |
| A4-4 | OneDrive provider via Microsoft Graph API v1.0 + OAuth 2.0 PKCE | `cloud/providers/onedrive.rs` (440 LOC, 5 tests) | ✅ |
| A4-5 | WebDAV provider via `opendal` (Nextcloud, ownCloud, Synology) | `cloud/providers/webdav.rs` (230 LOC, 3 tests) | ✅ |
| A4-6 | SFTP provider via `opendal` (any Linux server with SSH) | `cloud/providers/sftp.rs` (215 LOC, 1 test) | ✅ |
| A4-7 | OAuth token manager: PKCE flow, token refresh, keychain storage, 3 provider configs | `cloud/oauth.rs` (450 LOC, 8 tests) | ✅ |

### A5. Frontend UI ✅ COMPLETE

| # | Task | File | Status |
|---|------|------|--------|
| A5-1 | `StorageTab.tsx`: storage mode toggle, breakdown, provider selector, recovery key, migrate | `settings/StorageTab.tsx` | ✅ |
| A5-2 | `StorageBreakdown` component: visual bar chart by category | `settings/StorageTab.tsx` (inline) | ✅ |
| A5-3 | `CloudProviderPicker`: provider cards with connect/configure CTAs | `settings/StorageTab.tsx` (inline) | ✅ |
| A5-4 | `S3ConfigForm`: endpoint, bucket, access key, secret key, region, "Test Connection" | `settings/StorageTab.tsx` (inline) | ✅ |
| A5-5 | `MigrationProgressDialog`: modal with progress bar, per-phase checklist, speed, ETA, cancel | `settings/StorageTab.tsx` (inline) | ✅ |
| A5-6 | `RecoveryKeyPanel`: show/hide/copy recovery key, import key for new device | `settings/StorageTab.tsx` (inline) | ✅ |
| A5-7 | Add `StorageTab` to settings sidebar + router (lazy-loaded) | `SettingsSidebar.tsx`, `SettingsPages.tsx` | ✅ |
| A5-8 | `useCloudStatus` hook: polls `cloud_get_status`, listens `cloud_migration_progress` events | `hooks/use-cloud-status.ts` | ✅ |
| A5-9 | `CloudSyncIndicator` in sidebar (idle / syncing / migration progress) | `navigation/CloudSyncIndicator.tsx` | ✅ |

### A6. Tests & Polish ✅ COMPLETE

| # | Task | Status |
|---|------|--------|
| A6-1 | Unit tests: AES-256-GCM roundtrip, HKDF key derivation, file format structure, zstd compression, nonce uniqueness, recovery key validation — **18 tests** | ✅ |
| A6-2 | Unit tests: manifest serialization, SHA-256 known vectors, JSON stability, schema guard edge cases, file grouping, statistics accumulation — **14 tests** | ✅ |
| A6-3 | Integration test: full encrypt+upload+download+decrypt roundtrip with MockProvider, 7 file types, 100-file bulk test, wrong-key security invariant — **6 tests** | ✅ |
| A6-4 | Integration test: schema migration on restore — old archive (v5) → new app (v12) forward-compat, backward-compat rejection, boundary test — **2 tests** | ✅ |
| A6-5 | Integration test: migration resume after crash — partial upload (5/10), resume, full restore, idempotent reupload — **2 tests** | ✅ |
| A6-6 | macOS App Nap guard: ref-counted RAII guard via `AtomicUsize`, prevents throttling during sync — **2 tests** | ✅ |
| A6-7 | Network quality detection: latency probing, `SyncStrategy` enum (FullSync/DeferLargeFiles/OfflineQueue), connection type detection — **9 tests** | ✅ |

---

## Dependency Map

```
Work Stream C (Wire Cloud Backends):  ✅ COMPLETE
  C1 (TTS) ✅ · C2 (STT) ✅ · C3 (Diff) ✅ · C4 (Badges) ✅
  C5 (Specta) ✅

Work Stream D (Model Discovery):      ✅ COMPLETE
  D1 (Registry) ✅ · D2 (12 providers) ✅ · D3 (Frontend) ✅ · D4 (Context) ✅

Work Stream A (Cloud Storage):        ✅ COMPLETE (ALL TIERS)
  ✅ A1 (Foundation)   — encryption, manifest, provider trait, S3, commands
  ✅ A2 (Migration)    — full lifecycle + background sync engine
  ✅ A3 (FileStore)    — unified I/O layer + cloud backend with upload queue
  ✅ A4 (7 Providers)  — S3, iCloud, GDrive, Dropbox, OneDrive, WebDAV, SFTP
  ✅ A5 (Frontend UI)  — StorageTab, MigrationDialog, CloudSyncIndicator
  ✅ A6 (Tests/Polish) — 88 tests, App Nap guard, network quality detection
```

---

## Open Decisions — ✅ All Resolved

| # | Question | Resolution | Evidence |
|---|----------|------------|----------|
| 1 | **TTS return format** | Bare base64 string (not `{ audio, format }`) | `tts.rs` L81: returns `general_purpose::STANDARD.encode()`. Frontend `decodeAudioData()` handles MP3/PCM transparently. |
| 2 | **Default STT backend** | Local (Whisper) — cloud as opt-in | `tts.rs` L55: cloud only used when `router.tts_backend()` returns `Some` |
| 3 | **Default embedding backend** | Local (llama-server) — cloud as opt-in | `inference/router.rs`: embedding backend starts as `None`, activated only when user configures |
| 4 | **Diffusion provider names** | Kept `"nano-banana"` / `"nano-banana-pro"` | `ImagineGeneration.tsx` L8, L26, L64 — still uses original names. Recommendation not implemented. |
| 5 | **Model discovery cache TTL** | 30 minutes (as recommended) | `inference/model_discovery/mod.rs` L53: `CACHE_TTL = Duration::from_secs(30 * 60)` |
| 6 | **Discovery trigger** | First open per session + manual refresh | `use-cloud-models.ts`: calls `discover_cloud_models` on mount, exposes `refreshProvider` |
| 7 | **`model-library.ts` migration** | Kept as offline fallback | Cloud entries remain; `useCloudModels` deduplicates against them |
| 8 | **Cloud storage + cloud inference** | Independent (as recommended) | `cloud/` and `inference/` are separate modules with separate config keys |
| 9 | **fal.ai polling timeout** | 120s (as recommended) | `cloud_fal.rs` L96-98: `Duration::from_secs(120)` |
| 10 | **`selected_cloud_model` ownership** | Both (as recommended) | `OpenClawConfig.selected_cloud_model` + `UserConfig.selected_model_context_size` |
| 11 | **Image gen tool backend** | Always `diffusion_backend` from InferenceRouter | `imagine.rs`: `generate_with_cloud_backend()` routes through InferenceRouter |

