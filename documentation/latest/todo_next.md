# Scrappy — Next Sprint TODO

> **Author:** AI-assisted planning, 2026-02-26  
> **Scope:** Two major work streams: (A) Cloud Storage backend and (B) Full local/cloud inference separation — covering chat, embedding, TTS, STT, and image diffusion across all supported cloud providers.

---

## Table of Contents

1. [Work Stream A — Cloud Storage Implementation](#work-stream-a--cloud-storage-implementation)
   - A1. [Foundation](#a1-foundation)
   - A2. [Migration Engine](#a2-migration-engine)
   - A3. [File Access Abstraction (FileStore)](#a3-file-access-abstraction-filestore)
   - A4. [Additional Cloud Providers](#a4-additional-cloud-providers)
   - A5. [Frontend (Storage Settings UI)](#a5-frontend-storage-settings-ui)
   - A6. [Tests & Polish](#a6-tests--polish)
2. [Work Stream B — Full Local / Cloud Inference Separation](#work-stream-b--full-local--cloud-inference-separation)
   - B1. [Architecture: InferenceRouter — the Central Abstraction](#b1-architecture-inferencerouter--the-central-abstraction)
   - B2. [Chat Inference — Cloud Providers](#b2-chat-inference--cloud-providers)
   - B3. [Embedding — Cloud Providers](#b3-embedding--cloud-providers)
   - B4. [Text-to-Speech (TTS) — Cloud Providers](#b4-text-to-speech-tts--cloud-providers)
   - B5. [Speech-to-Text (STT) — Cloud Providers](#b5-speech-to-text-stt--cloud-providers)
   - B6. [Image Diffusion — Cloud Providers](#b6-image-diffusion--cloud-providers)
   - B7. [Frontend — Inference Mode UI & Models Browser](#b7-frontend--inference-mode-ui--models-browser)
   - B8. [Cross-Modal Consistency (Secrets, Config, OpenClaw)](#b8-cross-modal-consistency-secrets-config-openclaw)
   - B9. [Cloud Provider Model Discovery — Automatic & Live](#b9-cloud-provider-model-discovery--automatic--live)
3. [Dependency Map](#dependency-map)
4. [Open Decisions](#open-decisions)

---

## Work Stream A — Cloud Storage Implementation

> **Reference design:** `documentation/latest/cloud_storage_implementation.md` — full architecture, provider taxonomy, encryption design, migration flows, FileStore abstraction, OAuth guide, TypeScript bindings, cost transparency tables, offline behavior, and sync status UI. Read that document first before implementing any item below.

### A1. Foundation

| # | Task | Files | Priority |
|---|------|-------|----------|
| A1-1 | Add `opendal` (v0.55+), `aes-gcm`, `hkdf`, `zstd` to `backend/Cargo.toml` | `backend/Cargo.toml` | **P0** |
| A1-2 | Create `backend/src/cloud/` module tree (`mod.rs`, `provider.rs`, `encryption.rs`, `snapshot.rs`, `manifest.rs`, `progress.rs`) | `backend/src/cloud/` | **P0** |
| A1-3 | Implement `CloudProvider` trait (see §6.2 of reference doc) | `cloud/provider.rs` | **P0** |
| A1-4 | Implement S3-compatible provider — covers AWS S3, Cloudflare R2, Backblaze B2, Wasabi, DigitalOcean Spaces, MinIO | `cloud/providers/s3.rs` | **P0** |
| A1-5 | Implement AES-256-GCM envelope encryption with HKDF-SHA256 per-file key derivation (see §9.2) | `cloud/encryption.rs` | **P0** |
| A1-6 | Implement SQLite snapshot via `VACUUM INTO` + `PRAGMA wal_checkpoint(TRUNCATE)` (see §8.1) | `cloud/snapshot.rs` | **P0** |
| A1-7 | Write and attach SQLite migration: `cloud_config` + `cloud_migrations` tables (see §6.4) | `backend/migrations/` | **P0** |
| A1-8 | Implement `ArchiveManifest` — JSON serialization, SHA-256 per file, schema version guard (see §6.3) | `cloud/manifest.rs` | **P0** |
| A1-9 | Write SQLite migration: normalize all stored paths from absolute to relative (see §14.4) — prerequisite for migration | `backend/migrations/` | **P0** |

### A2. Migration Engine

| # | Task | Files | Priority |
|---|------|-------|----------|
| A2-1 | Implement `migrate_to_cloud()` — full local→cloud flow: preflight, DB snapshot, encrypt+upload, verify, mode switch, local cleanup (see §7.1) | `cloud/mod.rs` | **P0** |
| A2-2 | Implement `migrate_to_local()` — full cloud→local flow: preflight, download+decrypt, DB restore, vector rebuild fallback, mode switch, cloud cleanup option (see §7.2) | `cloud/mod.rs` | **P0** |
| A2-3 | Implement progress reporting — emit `cloud_migration_progress` Tauri events with phase, file counts, byte counts, speed, ETA (see §17.2) | `cloud/progress.rs` | **P0** |
| A2-4 | Register all cloud Tauri commands in `lib.rs` (see §17.1): `cloud_get_status`, `cloud_get_storage_breakdown`, `cloud_test_connection`, `cloud_migrate_to_cloud`, `cloud_migrate_to_local`, `cloud_cancel_migration`, `cloud_get_migration_progress`, `cloud_ensure_local`, `cloud_get_recovery_key`, `cloud_import_recovery_key`, `cloud_start_oauth`, `cloud_sync_now` | `backend/src/lib.rs` | **P0** |
| A2-5 | Implement migration resume on launch — detect `cloud_migrations.status = 'in_progress'` on startup, offer resume (see §11.1) | `cloud/mod.rs` | **P1** |
| A2-6 | Implement periodic background sync (cloud mode active) — 5-minute batch: WAL checkpoint → VACUUM INTO → encrypt → upload; queue new files (see §7.3) | `cloud/sync.rs` | **P1** |
| A2-7 | Store cloud master key in macOS Keychain (`com.scrappy.cloud-key`) using `security-framework` — already a dependency | `cloud/encryption.rs` | **P0** |

### A3. File Access Abstraction (FileStore)

> **Critical refactor** — required before cloud mode works. See §14 of reference doc for the full analysis of ~30 direct `std::fs` call sites that break in cloud mode.

| # | Task | Files | Priority |
|---|------|-------|----------|
| A3-1 | Implement `FileStore` struct as Tauri managed state (see §14.2): `write()`, `read()`, `ensure_local()`, `delete()`, `exists()`, `local_path()`, `list()` | `backend/src/file_store.rs` | **P0** |
| A3-2 | Phase A — local-only `FileStore` implementation: `write()` → `tokio::fs::write`, `read()` → `tokio::fs::read`. No behavior change, zero risk. Add `app.manage(FileStore::new(...))`. | `backend/src/file_store.rs`, `lib.rs` | **P0** |
| A3-3 | Migrate `images.rs` to use `FileStore` — 6 call sites: `upload_image`, `get_image_path`, `load_image_as_base64` | `backend/src/images.rs` | **P1** |
| A3-4 | Migrate `imagine.rs` to use `FileStore` — 4 call sites: generated image save, delete | `backend/src/imagine.rs` | **P1** |
| A3-5 | Migrate `rag.rs` to use `FileStore` — 2 call sites: document upload | `backend/src/rag.rs` | **P1** |
| A3-6 | Migrate `projects.rs` to use `FileStore` — 2 call sites: `delete_document` | `backend/src/projects.rs` | **P1** |
| A3-7 | Migrate `history.rs` to use `FileStore` — 2 call sites: docs/images cleanup in `delete_all_history` | `backend/src/history.rs` | **P1** |
| A3-8 | Migrate `openclaw/commands/sessions.rs` to use `FileStore` — 8 call sites for session file I/O | `backend/src/openclaw/commands/sessions.rs` | **P2** |
| A3-9 | Phase C — add cloud backend to `FileStore`: `write()` → local + queue upload; `read()` → local cache || cloud download | `backend/src/file_store.rs` | **P1** |
| A3-10 | Implement `CloudImage` frontend wrapper component as drop-in `<img>` replacement for cloud mode lazy-loading (see §15.3) | `frontend/src/components/CloudImage.tsx` | **P1** |
| A3-11 | Replace raw `<img>` with `<CloudImage>` in: `ImagineGallery.tsx` (×2), `ImagineGeneration.tsx` (×2), `MessageBubble.tsx` (×1), `ImagineView.tsx` (×1), `ChatProvider.tsx` (×1) | Various frontend files | **P1** |

### A4. Additional Cloud Providers

| # | Task | Files | Priority |
|---|------|-------|----------|
| A4-1 | Implement iCloud Drive provider via `NSFileManager.URLForUbiquityContainerIdentifier` (macOS-only, requires Apple Developer entitlement) | `cloud/providers/icloud.rs` | **P1** |
| A4-2 | Implement Google Drive provider via Drive API v3 + OAuth 2.0 PKCE loopback flow (see §16.2) | `cloud/providers/gdrive.rs` | **P1** |
| A4-3 | Implement Dropbox provider via Dropbox API v2 + OAuth 2.0 PKCE | `cloud/providers/dropbox.rs` | **P2** |
| A4-4 | Implement OneDrive provider via Microsoft Graph API + OAuth 2.0 PKCE | `cloud/providers/onedrive.rs` | **P2** |
| A4-5 | Implement WebDAV provider (Nextcloud, ownCloud, Synology) via `opendal` WebDAV backend | `cloud/providers/webdav.rs` | **P2** |
| A4-6 | Implement SFTP provider via `opendal` SFTP backend (any Linux server) | `cloud/providers/sftp.rs` | **P3** |
| A4-7 | Implement OAuth token manager: PKCE flow, `OAuthTokens` struct, token refresh, Keychain storage under `com.scrappy.oauth.{provider}` (see §16.3–16.4) | `cloud/oauth.rs` | **P1** |
| A4-8 | Register Scrappy developer apps with Google Cloud Console, Dropbox Developer, Microsoft Azure for embedded client IDs | External / build config | **P1** |

### A5. Frontend (Storage Settings UI)

| # | Task | Files | Priority |
|---|------|-------|----------|
| A5-1 | Create `StorageTab.tsx` — storage mode toggle (local / cloud), storage breakdown bar chart, provider selector, recovery key panel, migrate button (see §10.1) | `frontend/src/components/settings/StorageTab.tsx` | **P0** |
| A5-2 | Create `StorageBreakdown.tsx` — visual bar chart of storage by category (Generated Images, Documents, Chat Images, Database, Vectors, Other) with byte counts | `frontend/src/components/settings/StorageBreakdown.tsx` | **P1** |
| A5-3 | Create `CloudProviderPicker.tsx` — provider selection cards (iCloud, Google Drive, Dropbox, OneDrive, S3-Compatible, WebDAV, SFTP) with connect/configure CTAs | `frontend/src/components/settings/CloudProviderPicker.tsx` | **P0** |
| A5-4 | Create `S3ConfigForm.tsx` — endpoint URL, bucket, access key, secret key, region inputs; "Test Connection" button | `frontend/src/components/settings/S3ConfigForm.tsx` | **P0** |
| A5-5 | Create `MigrationProgress.tsx` — full-screen dialog: overall progress bar, per-phase checklist (DB / Docs / Images / Vectors / Other), speed, ETA, cancel button (see §10.2) | `frontend/src/components/settings/MigrationProgress.tsx` | **P0** |
| A5-6 | Create `RecoveryKeyPanel.tsx` — show/hide recovery key, copy to clipboard, rotate key button | `frontend/src/components/settings/RecoveryKeyPanel.tsx` | **P1** |
| A5-7 | Add `StorageTab` to settings sidebar/router (alongside Secrets, Gateway, Models, etc.) | `frontend/src/components/settings/SettingsPages.tsx` | **P0** |
| A5-8 | Create `useCloudStatus` hook — polls `cloud_get_status` every 30s + listens to `cloud_sync_status` events (see §21.2) | `frontend/src/hooks/use-cloud-status.ts` | **P1** |
| A5-9 | Add persistent cloud sync status indicator to sidebar/status bar (idle / syncing / error / offline) (see §21.1) | Sidebar component | **P2** |
| A5-10 | Implement cost estimate display in `CloudProviderPicker`: `estimated_monthly_cost` based on data size × hardcoded pricing table (see §19.1–19.2) | `CloudProviderPicker.tsx` | **P2** |
| A5-11 | Implement provider-to-provider switching flow (cloud → cloud transfer, presented as single "Switch Provider" operation) (see §18) | `StorageTab.tsx`, `cloud/mod.rs` | **P2** |

### A6. Tests & Polish

| # | Task | Priority |
|---|------|----------|
| A6-1 | Unit tests: AES-256-GCM encrypt/decrypt roundtrip, per-file key derivation, encrypted file format header | **P0** |
| A6-2 | Unit tests: manifest serialization/deserialization, SHA-256 integrity check, schema version guard | **P0** |
| A6-3 | Integration test: local → cloud → local roundtrip using MinIO in Docker (mimic realistic S3 transfer) | **P0** |
| A6-4 | Integration test: schema migration on restore — simulate old DB snapshot + newer app version | **P1** |
| A6-5 | Integration test: migration resume after simulated midpoint crash | **P1** |
| A6-6 | macOS App Nap guard during active sync: `NSProcessInfo.beginActivity(options: .userInitiated)` | **P2** |
| A6-7 | Network quality detection via `SCNetworkReachability` — `SyncStrategy` enum (FullSync / MeteredSync / DeferredSync) | **P2** |

---

## Work Stream B — Full Local / Cloud Inference Separation

### Overview

Currently, Scrappy has a partial local/cloud split:
- **Chat:** `UnifiedProvider` handles cloud providers (Anthropic, OpenAI, Gemini, Groq, OpenRouter) and local inference via `LlamaProvider`/`InferenceEngine`. This works but the routing lives ad-hoc in `chat.rs::resolve_provider()`.
- **Embedding:** Local-only (llama-server or MLX embed server). No cloud embedding provider is supported.
- **TTS:** Local-only (Piper sidecar). No cloud TTS.
- **STT:** Local-only (whisper-server sidecar). No cloud STT.
- **Diffusion:** Hybrid — local (`sd.cpp`) or Gemini Imagen 3 only (`imagine.rs::generate_with_gemini()`). Not extensible.

**Goal:** A clean, unified `InferenceRouter` layer that routes every AI modality (chat, embedding, TTS, STT, diffusion) to either local sidecars or cloud provider APIs, configurable per-modality independently.

> #### ⚠️ Key Architecture Constraint: `SecretStore` is the Single Source of Truth for API Keys
>
> **`SecretStore`** (`backend/src/secret_store.rs`) is the **application-level** Tauri managed state for all API keys. It is a thin delegation wrapper — every call to `secret_store.get("anthropic")` calls `keychain::get_key("anthropic")`, which reads the live `Mutex<HashMap>` in `keychain.rs`. This cache is backed by the macOS Keychain (encrypted at rest).
>
> **`OpenClawConfig`** is an **agent engine** concern. It is **NOT** the key store. `OpenClawConfig::new()` (in `identity.rs` lines 172–240) **copies** key values from `keychain::get_key()` into struct fields at construction time. These copies are **snapshots** — they go stale if keys change after construction. This is fine for the engine (which reads config once), but **wrong** for application code that needs live key reads.
>
> **Existing bugs** (currently read keys from `OpenClawConfig` but SHOULD use `SecretStore`):
> - `chat.rs::resolve_provider()` — calls `openclaw.get_config().await.anthropic_api_key` (line 79)
> - `imagine.rs::generate_with_gemini()` — calls `openclaw_mgr.get_config().await.gemini_api_key` (line 80)
>
> **Correct consumers of `SecretStore`** (already fixed, use as reference):
> - `hf_hub.rs` — `app.try_state::<SecretStore>()` (line 61)
> - `config.rs::get_hf_token()` — `app.try_state::<SecretStore>()` (line 315)
> - `model_manager.rs` — `app.try_state::<SecretStore>()` (line 305)
>
> **`OpenClawConfig` legitimate uses** (inside `openclaw/` module only):
> - `engine.rs::write_config()` — generates `auth-profiles.json` for the OpenClaw agent engine
> - `gateway.rs` — reports provider status (`has_anthropic_key = true`)
> - `keys.rs` — Tauri commands for saving/deleting keys (writes to keychain, updates `OpenClawConfig` copy)
>
> ```
> macOS Keychain (encrypted blob)
>       ↕ (one read on startup, one write per mutation)
> keychain.rs — KEY_CACHE: Mutex<HashMap<String, String>>  ← SINGLE SOURCE OF TRUTH
>       ↕ (live reads)                ↕ (snapshot at construction)
>   SecretStore                  OpenClawConfig
>   .get("anthropic")            .anthropic_api_key = keychain::get_key("anthropic")
>   → keychain::get_key()        → copied into struct field ONCE at ::new()
>   → always returns live         → stale if key changes after construction
>   value from cache              → ONLY for openclaw/ module (engine config)
>       ↕
>   App-wide consumers:
>   ├── InferenceRouter (B1)     ← NEW: read keys here
>   ├── CloudModelRegistry (B9)  ← NEW: read keys here
>   ├── hf_hub.rs                ← ALREADY correct
>   ├── config.rs (HF token)     ← ALREADY correct
>   └── model_manager.rs         ← ALREADY correct
>
>   BUGS to fix:
>   ├── chat.rs::resolve_provider()        → B2-3: switch to SecretStore
>   └── imagine.rs::generate_with_gemini() → B6-8: switch to SecretStore
> ```

---

### B1. Architecture: InferenceRouter — the Central Abstraction

This is the foundational refactor that makes everything else possible.

```
backend/src/inference/
├── mod.rs              ← pub use; InferenceRouter factory + Tauri state
├── router.rs           ← InferenceRouter struct: routing decisions per modality
├── provider_kind.rs    ← ModalityProvider enum (one per modality)
├── chat/
│   ├── mod.rs          ← ChatBackend trait
│   ├── local.rs        ← LocalChatBackend (wraps existing InferenceEngine)
│   └── cloud.rs        ← CloudChatBackend (wraps UnifiedProvider)
├── embedding/
│   ├── mod.rs          ← EmbeddingBackend trait
│   ├── local.rs        ← LocalEmbeddingBackend (llama-server / mlx-embed)
│   ├── openai.rs       ← OpenAI text-embedding-3-* API
│   ├── gemini.rs       ← Gemini text-embedding-004 API
│   ├── voyager.rs      ← Voyage AI embedding API
│   └── cohere.rs       ← Cohere embed-v3 API
├── tts/
│   ├── mod.rs          ← TtsBackend trait
│   ├── local.rs        ← LocalTtsBackend (Piper sidecar, existing tts.rs)
│   ├── openai.rs       ← OpenAI tts-1 / tts-1-hd API
│   ├── elevenlabs.rs   ← ElevenLabs TTS API
│   └── gemini.rs       ← Gemini TTS (if/when available)
├── stt/
│   ├── mod.rs          ← SttBackend trait
│   ├── local.rs        ← LocalSttBackend (whisper-server, existing stt.rs)
│   ├── openai.rs       ← OpenAI Whisper API (cloud)
│   ├── gemini.rs       ← Gemini Speech recognition API
│   └── deepgram.rs     ← Deepgram Nova-3 API
└── diffusion/
    ├── mod.rs          ← DiffusionBackend trait
    ├── local.rs        ← LocalDiffusionBackend (sd.cpp sidecar, existing image_gen.rs)
    ├── dalle.rs        ← OpenAI DALL-E 3 API
    ├── imagen.rs       ← Google Imagen 3 API (extract from imagine.rs)
    ├── stability.rs    ← Stability AI (stable-image/generate/*)
    ├── fal.rs          ← fal.ai (FLUX, SDXL hosted)
    └── together.rs     ← Together AI diffusion API
```

| # | Task | Files | Priority |
|---|------|-------|----------|
| B1-1 | Define `ChatBackend`, `EmbeddingBackend`, `TtsBackend`, `SttBackend`, `DiffusionBackend` traits with async methods and a `BackendInfo { name, is_local, model_id }` return type | `inference/{chat,embedding,tts,stt,diffusion}/mod.rs` | **P0** |
| B1-2 | Implement `InferenceRouter` struct as Tauri-managed state — holds one active backend per modality, loaded from `UserConfig`. Cloud backends receive keys from `SecretStore` (not `OpenClawConfig`). `InferenceRouter::new()` takes `SecretStore` reference. | `inference/router.rs` | **P0** |
| B1-3 | Add per-modality routing config to `UserConfig` in `config.rs`: `chat_backend`, `embedding_backend`, `tts_backend`, `stt_backend`, `diffusion_backend` (each = `"local"` \| `"openai"` \| `"gemini"` \| etc.). **Deprecate** `selected_chat_provider` — migrate to `chat_backend` with backward-compatible fallback. | `backend/src/config.rs` | **P0** |
| B1-4 | Register `InferenceRouter` as Tauri state in `lib.rs`; wire up Tauri command `get_inference_backends()` → returns active + available backends per modality | `backend/src/lib.rs` | **P0** |
| B1-5 | Add `update_inference_backend(modality, backend_id)` Tauri command to hot-swap backends at runtime without restart | `backend/src/lib.rs` or `inference/mod.rs` | **P1** |

### B2. Chat Inference — Cloud Providers

The chat path is already partially cloud-capable via `UnifiedProvider`. The primary work here is making the routing clean and extensible.

> **Key retrieval:** `resolve_provider()` currently reads keys from `OpenClawConfig` (legacy). After B2-3, it must read from `SecretStore` via `State<'_, SecretStore>`. See the architecture constraint note above.

| # | Task | Files | Priority |
|---|------|-------|----------|
| B2-1 | Wrap existing `UnifiedProvider` as `CloudChatBackend` implementing `ChatBackend` | `inference/chat/cloud.rs` | **P0** |
| B2-2 | Wrap existing `InferenceEngine` + `LlamaProvider` as `LocalChatBackend` implementing `ChatBackend` | `inference/chat/local.rs` | **P0** |
| B2-3 | **Refactor `chat.rs::resolve_provider()`** — (a) replace `openclaw.get_config()` **key reads** with `secret_store.get("anthropic")` etc. via `State<'_, SecretStore>`, (b) delegate routing to `InferenceRouter::chat_backend()` instead of ad-hoc match. API keys come from `SecretStore`. Model selection (`selected_cloud_model`) is a non-secret — it can stay on `OpenClawIdentity` for now, or migrate to `UserConfig.chat_model` (see Open Decision #14). Context size from discovered model metadata (B9-30). The `OpenClawManager` dependency is removed from the **key retrieval** path entirely — `resolve_provider()` no longer needs it for keys. | `backend/src/chat.rs` | **P0** |
| B2-4 | Expand `ProviderKind` in `unified_provider.rs` to add: `Mistral`, `xAI` (Grok), `Amazon Bedrock`, `Cohere`, `Together`, `Venice`, `Moonshot`, `MiniMax`, `NVIDIA`, `Qianfan`, `Xiaomi` — all implicit providers that are already in `keychain::PROVIDERS` and `SecretsTab.tsx` but not in the Rig chat path. Most reuse the OpenAI-compat streaming path; Bedrock needs its own `stream_bedrock()` via AWS SDK. | `backend/src/rig_lib/unified_provider.rs` | **P1** |
| B2-5 | Add `resolve_provider()` match arms for all new providers — each uses `secret_store.get("<provider_id>")` for key retrieval and the correct `base_url` (e.g., Venice → `https://api.venice.ai/api/v1`, Moonshot → `https://api.moonshot.ai/v1`, NVIDIA → `https://integrate.api.nvidia.com/v1`). Context size should read from discovered model metadata when available, with a sensible default fallback. | `backend/src/chat.rs` | **P1** |
| B2-6 | Update `model-library.ts` with model entries for new providers (Mistral, xAI Grok, Together, Cohere) to match expanded `ProviderKind` | `frontend/src/lib/model-library.ts` | **P1** |
| B2-7 | Implement `stream_bedrock()` in `unified_provider.rs` — Bedrock is NOT OpenAI-compatible, it uses AWS SDK `InvokeModelWithResponseStream`. Requires `aws-sdk-bedrockruntime` crate. Credentials from `SecretStore`: `bedrock_access_key_id`, `bedrock_secret_access_key`, `bedrock_region`. | `backend/src/rig_lib/unified_provider.rs`, `Cargo.toml` | **P2** |
| B2-8 | Token counting for cloud providers: replace `len/3` fallback with provider-native `/tokenize` endpoints where available (OpenAI `tiktoken` API, Anthropic `count_tokens`, Gemini `countTokens`) | `rig_lib/llama_provider.rs` | **P2** |
| B2-9 | Define shared `PROVIDER_ENDPOINTS` constant — maps `provider_id` → `{ base_url, api_compat, default_model, default_context_size }`. Used by both `resolve_provider()` and `model_discovery/` modules to eliminate base_url duplication. | `backend/src/inference/provider_endpoints.rs` | **P1** |

**Supported cloud chat providers (target):**

| Provider | API Standard | Auth | Notes |
|----------|-------------|------|-------|
| Anthropic | Anthropic Messages API | API key (Keychain) | Already implemented ✅ |
| OpenAI | OpenAI API | API key | Already implemented ✅ |
| Google Gemini | Gemini REST API | API key | Already implemented ✅ |
| Groq | OpenAI-compat | API key | Already implemented ✅ |
| OpenRouter | OpenAI-compat | API key | Already implemented ✅ |
| Mistral AI | OpenAI-compat | API key | Add to ProviderKind |
| xAI (Grok) | OpenAI-compat | API key | Add to ProviderKind |
| Together AI | OpenAI-compat | API key | Add to ProviderKind |
| Venice AI | OpenAI-compat | API key | Add to ProviderKind — privacy-focused, uncensored open-source models |
| Moonshot (Kimi) | OpenAI-compat | API key | Add to ProviderKind — long-context Kimi K2/K2.5 models |
| MiniMax | OpenAI-compat | API key | Add to ProviderKind — multimodal generation |
| NVIDIA NIM | OpenAI-compat | NGC API key | Add to ProviderKind — enterprise-grade NVIDIA-optimized inference |
| Baidu Qianfan | Baidu API | API key | Add handler — ERNIE models |
| Xiaomi | OpenAI-compat | API key | Add to ProviderKind — MiMo VL models |
| Cohere | Cohere API | API key | Unique API, add handler |
| Amazon Bedrock | AWS SDK | AWS credentials (Keychain) | OpenClaw already supports; wire Rig path |

### B3. Embedding — Cloud Providers

Currently, embedding is 100% local (llama-server or MLX). Cloud embedding unlocks:
- Embedding without downloading a local model
- Higher-quality embeddings for RAG (OpenAI text-embedding-3-large = 3072 dims)
- Cross-device consistent embeddings when using cloud storage mode

| # | Task | Files | Priority |
|---|------|-------|----------|
| B3-1 | Define `EmbeddingBackend` trait: `embed_batch(texts: Vec<String>) -> Vec<Vec<f32>>`, `dimensions() -> usize`, `model_name() -> &str` | `inference/embedding/mod.rs` | **P0** |
| B3-2 | Wrap existing llama-server / MLX embed as `LocalEmbeddingBackend` | `inference/embedding/local.rs` | **P0** |
| B3-3 | Implement `OpenAiEmbeddingBackend`: calls `POST /v1/embeddings` with `text-embedding-3-small` (1536 dims) or `text-embedding-3-large` (3072 dims) | `inference/embedding/openai.rs` | **P1** |
| B3-4 | Implement `GeminiEmbeddingBackend`: calls `POST /v1beta/models/text-embedding-004:embedContent` (768 dims) | `inference/embedding/gemini.rs` | **P1** |
| B3-5 | Implement `VoyageEmbeddingBackend`: calls Voyage AI REST API (`voyage-3` = 1024 dims, `voyage-3-large` = 1024 dims) — best-in-class for RAG | `inference/embedding/voyager.rs` | **P2** |
| B3-6 | Implement `CohereEmbeddingBackend`: calls `POST /v1/embed` with `embed-multilingual-v3.0` (1024 dims) | `inference/embedding/cohere.rs` | **P2** |
| B3-7 | Update `VectorStoreManager::reinit()` to accept new dimension when switching from local (e.g. 768 dims) to cloud (e.g. 3072 dims) — existing purge+reinit logic handles this ✅ | `backend/src/vector_store.rs` | **P0** |
| B3-8 | Update **both** embedding call sites in `rag.rs` to use `InferenceRouter::embedding_backend().embed_batch()` instead of directly calling `http://127.0.0.1:{port}/v1/embeddings`. Two sites: (a) `ingest_document()` at ~line 434 — embeds document chunks during ingestion, (b) `retrieve_context_internal()` at ~line 748 — embeds the user query for similarity search. Both currently hardcode the local llama-server URL. | `backend/src/rag.rs` | **P0** |
| B3-9 | Add `embedding_backend` + `embedding_model` config to `UserConfig`; expose in Settings → Models → Embedding section | `backend/src/config.rs`, Settings UI | **P0** |
| B3-10 | Guard: if active embedding backend changes dimensions, warn user that all vector indices must be rebuilt (offer one-click rebuild) | `backend/src/inference/router.rs`, Settings UI | **P1** |
| B3-11 | Add API keys for Voyage AI, Cohere to `SecretsTab.tsx` and Keychain integration in `secret_store.rs` | `frontend/src/components/settings/SecretsTab.tsx`, `backend/src/secret_store.rs` | **P1** |

**Supported cloud embedding providers (target):**

| Provider | Model | Dims | Cost (per 1M tokens) |
|----------|-------|------|---------------------|
| OpenAI | text-embedding-3-small | 1536 | $0.02 |
| OpenAI | text-embedding-3-large | 3072 | $0.13 |
| Google Gemini | text-embedding-004 | 768 | Free (within quota) |
| Voyage AI | voyage-3 | 1024 | $0.06 |
| Cohere | embed-multilingual-v3.0 | 1024 | $0.10 |

### B4. Text-to-Speech (TTS) — Cloud Providers

Currently, TTS is Piper-only (local one-shot sidecar). Cloud TTS unlocks higher-quality, multi-voice synthesis without a local model download.

| # | Task | Files | Priority |
|---|------|-------|----------|
| B4-1 | Define `TtsBackend` trait: `synthesize(text: &str, voice: &str) -> Vec<u8>` (returns raw PCM or MP3 bytes), `available_voices() -> Vec<VoiceInfo>`, `output_format() -> AudioFormat` | `inference/tts/mod.rs` | **P0** |
| B4-2 | Wrap existing Piper sidecar as `LocalTtsBackend` — migrate `tts.rs::tts_synthesize` to delegate to `InferenceRouter::tts_backend()` | `inference/tts/local.rs`, `backend/src/tts.rs` | **P0** |
| B4-3 | Implement `OpenAiTtsBackend`: calls `POST /v1/audio/speech` with model `tts-1` or `tts-1-hd`, voice (alloy/echo/fable/onyx/nova/shimmer), response format `pcm`/`mp3` | `inference/tts/openai.rs` | **P1** |
| B4-4 | Implement `ElevenLabsTtsBackend`: calls `POST /v1/text-to-speech/{voice_id}/stream` (websocket streaming), returns MP3 | `inference/tts/elevenlabs.rs` | **P2** |
| B4-5 | Implement `GeminiTtsBackend`: calls `POST /v1beta/models/gemini-2.5-flash-preview-tts:generateContent` with SSE streaming, voice selection via `speechConfig.voiceConfig`, returns PCM audio bytes. Free tier available. | `inference/tts/gemini.rs` | **P2** |
| B4-6 | Update `tts_synthesize` Tauri command to use `InferenceRouter::tts_backend()` — frontend API unchanged | `backend/src/tts.rs` | **P0** |
| B4-7 | Add `get_tts_voices` Tauri command: returns available voices for the active backend | `backend/src/tts.rs` | **P1** |
| B4-8 | Update `MessageBubble.tsx` "Read Aloud" button: add voice selector dropdown (populated from `get_tts_voices`) | `frontend/src/components/chat/MessageBubble.tsx` | **P2** |
| B4-9 | Add ElevenLabs API key to `SecretsTab.tsx` and Keychain | `frontend/src/components/settings/SecretsTab.tsx`, `backend/src/secret_store.rs` | **P2** |
| B4-10 | Handle audio format routing on frontend: OpenAI returns MP3, Gemini returns PCM, Piper returns raw PCM — ensure `AudioContext` decode path handles all formats via `decodeAudioData` | `frontend/src/components/chat/MessageBubble.tsx` | **P1** |

**Supported cloud TTS providers (target):**

| Provider | Model | Voices | Streaming | Cost |
|----------|-------|--------|-----------|------|
| OpenAI | tts-1 / tts-1-hd | 6 voices | No (single request) | $15–$30 / 1M chars |
| ElevenLabs | Eleven Multilingual v2 | 100s (cloned + stock) | Yes (WebSocket) | $0.18 / 1K chars (paid) |
| Google Gemini TTS | gemini-2.5-flash-preview-tts | — | SSE | Free tier available |

### B5. Speech-to-Text (STT) — Cloud Providers

Currently, STT is whisper-server only (local). Cloud STT provides better accuracy for non-English, no local model download, and real-time streaming transcription.

> **Note:** Adding auth to whisper-server remains blocked upstream (see TODO.md item). Cloud STT is the workaround.

| # | Task | Files | Priority |
|---|------|-------|----------|
| B5-1 | Define `SttBackend` trait: `transcribe(audio: &[u8], format: AudioFormat, language: Option<&str>) -> String`, `supports_streaming() -> bool`, `languages() -> Vec<&str>` | `inference/stt/mod.rs` | **P0** |
| B5-2 | Wrap existing whisper-server as `LocalSttBackend` — migrate `stt.rs` to delegate to `InferenceRouter::stt_backend()` | `inference/stt/local.rs`, `backend/src/stt.rs` | **P0** |
| B5-3 | Implement `OpenAiSttBackend`: calls `POST /v1/audio/transcriptions` with model `whisper-1`, language hint, returns JSON `{ text }` | `inference/stt/openai.rs` | **P1** |
| B5-4 | Implement `GeminiSttBackend`: calls Gemini `generateContent` with inline audio part (`data:audio/wav;base64,...`) — Gemini 2.0 Flash supports audio natively | `inference/stt/gemini.rs` | **P1** |
| B5-5 | Implement `DeepgramSttBackend`: calls `POST https://api.deepgram.com/v1/listen` with Nova-3 model — best accuracy for real-time (supports streaming WebSocket, start with batch) | `inference/stt/deepgram.rs` | **P2** |
| B5-6 | Update `stt_transcribe` Tauri command (and `use-audio-recorder.ts` in frontend) to use `InferenceRouter::stt_backend()` — no change to frontend API | `backend/src/stt.rs` | **P0** |
| B5-7 | Add Deepgram API key to `SecretsTab.tsx` and Keychain | `frontend/src/components/settings/SecretsTab.tsx`, `backend/src/secret_store.rs` | **P2** |
| B5-8 | Language selector in `ChatInput.tsx` STT button — shown when cloud backend active (cloud STT has broad language support vs. whisper.gguf model language coverage) | `frontend/src/components/chat/ChatInput.tsx` | **P2** |
| B5-9 | Guard: if STT backend is local and whisper model is not downloaded, auto-suggest switching to cloud STT in the onboarding flow | `frontend/src/components/onboarding/` | **P2** |

**Supported cloud STT providers (target):**

| Provider | Model | Languages | Streaming | Cost |
|----------|-------|-----------|-----------|------|
| OpenAI | Whisper v2-large | 57 languages | No (batch) | $0.006 / min |
| Google Gemini | Gemini 2.0 Flash | 100+ | SSE | Free tier available |
| Deepgram | Nova-3 | 30+ | WebSocket | $0.0043 / min |

### B6. Image Diffusion — Cloud Providers

Currently only two diffusion paths exist: local `sd.cpp` and Gemini Imagen 3 (hardcoded in `imagine.rs::generate_with_gemini()`). This needs proper abstraction.

> **Two separate call sites exist for image generation:**
>
> 1. **`imagine.rs`** — the Imagine Studio UI (`imagine_generate` Tauri command). Supports both local sd.cpp/mflux and Gemini Imagen 3.
> 2. **`image_gen.rs`** — the chat-context image generation tool (`generate_image` function, called by `ImageGenTool` in `rig_lib/tools/image_gen_tool.rs`). Currently **local-only** (sd.cpp/mflux). When the Rig agent decides to generate an image in chat, it uses this path — which ignores cloud diffusion backends entirely.
>
> Both call sites must route through `InferenceRouter::diffusion_backend()` after the refactor.

| # | Task | Files | Priority |
|---|------|-------|----------|
| B6-1 | Define `DiffusionBackend` trait: `generate(prompt, negative_prompt, width, height, steps, guidance, seed) -> Vec<u8>` (PNG bytes), `supports_negative_prompt() -> bool`, `max_resolution() -> (u32, u32)` | `inference/diffusion/mod.rs` | **P0** |
| B6-2 | Wrap existing `image_gen.rs` as `LocalDiffusionBackend` — migrate `imagine_generate` command to call through `InferenceRouter::diffusion_backend()` | `inference/diffusion/local.rs`, `backend/src/imagine.rs` | **P0** |
| B6-3 | **Extract** `generate_with_gemini()` from `imagine.rs` into `inference/diffusion/imagen.rs` as `ImagenDiffusionBackend` | `inference/diffusion/imagen.rs`, `backend/src/imagine.rs` | **P0** |
| B6-4 | Implement `DalleDiffusionBackend`: calls `POST /v1/images/generations` with `dall-e-3` model, returns `b64_json` | `inference/diffusion/dalle.rs` | **P1** |
| B6-5 | Implement `StabilityDiffusionBackend`: calls `POST /v2beta/stable-image/generate/core` (SDXL-turbo hosted), multipart form, returns PNG | `inference/diffusion/stability.rs` | **P1** |
| B6-6 | Implement `FalDiffusionBackend`: calls fal.ai queue API for `fal-ai/flux/dev` and `fal-ai/stable-diffusion-v3-medium` — handles async polling | `inference/diffusion/fal.rs` | **P2** |
| B6-7 | Implement `TogetherDiffusionBackend`: calls Together AI `POST /v1/images/generations` OpenAI-compat endpoint | `inference/diffusion/together.rs` | **P2** |
| B6-8 | **Refactor `imagine_generate` in `imagine.rs`** to route through `InferenceRouter::diffusion_backend()`. Remove the old `generate_with_gemini()` function which currently reads keys from `OpenClawConfig` (line 73–82: `openclaw_mgr.get_config().await.gemini_api_key`) — this is a **bug**, same as `chat.rs`. After refactor, the Imagen backend reads the Gemini key from `SecretStore` via the `InferenceRouter`. | `backend/src/imagine.rs` | **P0** |
| B6-9 | Add Stability AI, fal.ai, Together AI keys to `SecretsTab.tsx` and Keychain | `frontend/src/components/settings/SecretsTab.tsx`, `backend/src/secret_store.rs` | **P1** |
| B6-10 | Update `ImagineGeneration.tsx` provider picker to list all active diffusion backends | `frontend/src/components/imagine/ImagineGeneration.tsx` | **P1** |
| B6-11 | Add `get_diffusion_backends` Tauri command: returns list of available (configured + key present) diffusion backends | `backend/src/lib.rs` | **P1** |
| B6-12 | **Route `image_gen.rs::generate_image()` through `InferenceRouter::diffusion_backend()`** — this is the chat-context image generation path used by `ImageGenTool` (Rig agent tool). Currently local-only (sd.cpp/mflux). After this, when the agent generates an image in chat, it can use whichever diffusion backend is active (local or cloud). Update `ImageGenTool::call()` in `rig_lib/tools/image_gen_tool.rs` to obtain the diffusion backend from `InferenceRouter` instead of calling `generate_image()` directly. | `backend/src/image_gen.rs`, `backend/src/rig_lib/tools/image_gen_tool.rs` | **P0** |
| B6-13 | Guard: when `diffusion_backend` is cloud, the `ImageGenTool` and `imagine_generate` should both return the generated image URL/bytes and save to disk via `FileStore` — ensure consistent image storage path regardless of backend. | `backend/src/imagine.rs`, `backend/src/image_gen.rs` | **P1** |

**Supported cloud diffusion providers (target):**

| Provider | Model | Resolution | Cost per image |
|----------|-------|------------|---------------|
| Google (Imagen) | Imagen 3 | up to 1024×1024 | ~$0.04 | Already implemented (extract) |
| OpenAI | DALL-E 3 | 1024×1024/1792×1024 | $0.04–$0.12 |
| Stability AI | SDXL Core / Ultra | up to 1536×1536 | $0.03–$0.08 |
| fal.ai | FLUX Dev / SDXL | up to 2048×2048 | $0.025–$0.05 |
| Together AI | FLUX / SD3 | up to 1024×1024 | $0.02 |

### B7. Frontend — Inference Mode UI & Models Browser

| # | Task | Files | Priority |
|---|------|-------|----------|
| B7-1 | Create `InferenceModeTab.tsx` in Settings — one section per modality (Chat, Embedding, TTS, STT, Diffusion) with a backend selector dropdown and active model/voice selector | `frontend/src/components/settings/InferenceModeTab.tsx` | **P0** |
| B7-2 | Add `InferenceModeTab` to settings sidebar/router | `frontend/src/components/settings/SettingsPages.tsx` | **P0** |
| B7-3 | Local / Cloud badge in `ModelSelector` (chat view): show `☁️` when cloud backend active, `🖥️` when local | `frontend/src/components/chat/ModelSelector.tsx` | **P1** |
| B7-4 | Badge in STT button in `ChatInput.tsx`: `🖥️ Whisper` vs `☁️ OpenAI` etc. | `frontend/src/components/chat/ChatInput.tsx` | **P2** |
| B7-5 | Badge in TTS "Read Aloud" button in `MessageBubble.tsx` | `frontend/src/components/chat/MessageBubble.tsx` | **P2** |
| B7-6 | Badge in Imagine Studio provider section | `frontend/src/components/imagine/ImagineGeneration.tsx` | **P1** |
| B7-7 | Onboarding wizard step: "Choose your inference mode" — local (download required) vs cloud (API key required) vs hybrid | `frontend/src/components/onboarding/` | **P2** |
| B7-8 | Per-modality cost estimate in `InferenceModeTab.tsx` — estimated monthly cost at current usage rate for each cloud backend option | `InferenceModeTab.tsx` | **P2** |
| B7-9 | "Cloud only" mode badge in app title bar / sidebar when ALL modalities are cloud — for users who never want to download models | Sidebar or top nav | **P3** |
| B7-10 | **Models Browser source filter** — add a top-level filter bar to `ModelBrowser.tsx` with tabs: **All** / **🖥️ Local** / **☁️ Cloud** that filters the model list by source (local downloads + HF discovery vs. cloud provider API models). Filter state persists across tab switches. | `frontend/src/components/settings/ModelBrowser.tsx` | **P0** |
| B7-11 | **Models Browser category filter** — add sub-filter chips or tabs below the source filter: **Chat** / **Embedding** / **TTS** / **STT** / **Diffusion** / **All**. Chips map to `ExtendedModelDefinition.category` for local models and `CloudModelEntry.modality` for cloud models. Both filters compose (e.g. "Cloud + Embedding" shows only cloud embedding models). | `frontend/src/components/settings/ModelBrowser.tsx` | **P0** |
| B7-12 | **Cloud models section in ModelBrowser** — when the ☁️ Cloud source filter is active, render cloud-discovered models grouped by provider. Each model card shows: model name, provider badge, modality tags, context window (if chat), pricing, and a "Select" button that sets it as the active model for the corresponding modality. No download needed. | `frontend/src/components/settings/ModelBrowser.tsx`, new `CloudModelCard.tsx` | **P0** |
| B7-13 | **Unified model list** — when "All" source filter is active, merge local `MODEL_LIBRARY` entries + HF discovered models + cloud-discovered models into a single list, sorted by category → provider → name. Local models show download status, cloud models show "Ready" badge. | `frontend/src/components/settings/ModelBrowser.tsx` | **P1** |
| B7-14 | **Migrate `CloudBrainConfigModal.tsx` `PROVIDER_MODELS`** — currently hardcodes model lists per provider (lines 14–99). After B9, replace with live data from `useCloudModels()` hook. The hardcoded list is 99 lines of model entries that go stale on every provider release. The modal should display discovered models, filtered by `ModalityCategory::Chat`, for the selected provider. Keep hardcoded entries as fallback for offline use (same as `model-library.ts` strategy). This modal is the OpenClaw Cloud Brain config — the model allowlist should be computed from discovery, not maintained manually. | `frontend/src/components/openclaw/CloudBrainConfigModal.tsx` | **P1** |

### B8. Cross-Modal Consistency (Secrets, Config, OpenClaw)

> **All key storage goes through `SecretStore`** which wraps `keychain.rs`. New providers are added to `keychain::PROVIDERS` (the slug list), `SecretStore` convenience accessors, and the frontend `SecretsTab.tsx`.

| # | Task | Files | Priority |
|---|------|-------|----------|
| B8-1 | Add missing API key slots to `SecretsTab.tsx`: Voyage AI, Deepgram, ElevenLabs, Stability AI, fal.ai — these providers don't have `SecretCard` entries yet. Together AI already has a card. | `frontend/src/components/settings/SecretsTab.tsx` | **P0** |
| B8-2 | Add new provider slugs to `keychain::PROVIDERS`: `"voyage"`, `"deepgram"`, `"elevenlabs"`, `"stability"`, `"fal"`. Add convenience accessors to `SecretStore` (`voyage_key()`, `deepgram_key()`, etc.). Add save/delete Tauri commands in `keys.rs` using the existing implicit provider pattern (`openclaw_save_implicit_provider_key` already handles arbitrary slugs). No changes to `OpenClawConfig` struct needed — the engine reads keys from the keychain cache directly. | `backend/src/openclaw/config/keychain.rs`, `backend/src/secret_store.rs`, `backend/src/openclaw/commands/keys.rs` | **P0** |
| B8-3 | Wire new providers into OpenClaw `auth-profiles.json` generation — Voyage (embedding), Deepgram, ElevenLabs, Stability AI should be injectable as secrets for OpenClaw skills | `backend/src/openclaw/config/engine.rs` | **P1** |
| B8-4 | Extend `UserConfig` with: `embedding_backend`, `embedding_model`, `tts_backend`, `tts_voice`, `stt_backend`, `stt_language`, `diffusion_backend`, `diffusion_model` | `backend/src/config.rs` | **P0** |
| B8-5 | Generate `specta` TypeScript bindings for all new Tauri commands (`get_inference_backends`, `update_inference_backend`, `get_tts_voices`, `get_diffusion_backends`) and regenerate `bindings.ts` | `backend/src/lib.rs`, `frontend/src/lib/bindings.ts` | **P0** |
| B8-6 | Update `TECHNICAL_ARCHITECTURE.md` §4 (Managed State) and §4.3 (Core Modules) to document new `InferenceRouter` state, `SecretStore` as the key authority, and `inference/` module tree | `documentation/latest/TECHNICAL_ARCHITECTURE.md` | **P1** |
| B8-7 | Update `MICROSERVICES_AND_SIDECARS.md` §2a (InferenceEngine) to clarify it governs only local chat inference; link to new `InferenceRouter` for full picture | `documentation/latest/MICROSERVICES_AND_SIDECARS.md` | **P1** |

### B9. Cloud Provider Model Discovery — Automatic & Live

> **Problem:** `model-library.ts` (46 KB, 1114 lines) hardcodes every cloud model. When OpenAI releases GPT-5.2 or Anthropic ships Claude Sonnet 4.6, the app shows stale models until a code update. Every provider has an API to list available models — we should use them.
>
> **Solution:** A backend `CloudModelRegistry` that queries each provider's model listing API, caches results, categorizes models by modality (chat, embedding, TTS, STT, diffusion), and exposes them to the frontend via a Tauri command. The hardcoded `model-library.ts` becomes a **fallback + local model catalog only** — cloud models are always live.

#### How Model Discovery Works Per Provider

Each provider exposes a model listing API. The discovery layer calls these APIs, parses the response, and classifies each model into a modality category based on model ID patterns and metadata.

| Provider | List Models API | Auth | Returns | Modality Classification Strategy |
|----------|----------------|------|---------|-----------------------------------|
| **OpenAI** | `GET /v1/models` | `Authorization: Bearer sk-...` | JSON array of `{ id, created, owned_by }` | **ID pattern matching:** `gpt-*`, `o1-*`, `o3-*`, `o4-*` → Chat. `text-embedding-*` → Embedding. `tts-*` → TTS. `whisper-*` → STT. `dall-e-*` → Diffusion. Filter out deprecated/fine-tuned models by `owned_by` field. |
| **Anthropic** | `GET /v1/models` | `x-api-key: sk-ant-...` + `anthropic-version: 2023-06-01` | JSON array of `{ id, display_name, type, created_at }` | **All Chat** — Anthropic only offers chat/completion models. `type: "model"` filter. Match `claude-*` IDs. |
| **Google Gemini** | `GET /v1beta/models` | `?key=AIza...` | JSON array of `{ name, displayName, description, supportedGenerationMethods, inputTokenLimit, outputTokenLimit }` | **`supportedGenerationMethods` field:** `"generateContent"` → Chat. `"embedContent"` → Embedding. `"generateImage"` → Diffusion. Filter by `name` prefix: `models/gemini-*` for chat, `models/text-embedding-*` for embedding, `models/imagen-*` for diffusion. |
| **Groq** | `GET /openai/v1/models` | `Authorization: Bearer gsk_...` | JSON array (OpenAI-compat format) | **ID pattern matching:** `llama-*`, `mixtral-*`, `gemma-*` → Chat. `whisper-*` → STT. Groq does not offer embedding/TTS/diffusion. |
| **OpenRouter** | `GET /api/v1/models` | `Authorization: Bearer sk-or-...` | JSON array of `{ id, name, context_length, pricing, architecture }` | **All Chat** — OpenRouter is a chat model router. No embedding/TTS/STT/diffusion. Rich metadata: `context_length`, `pricing.prompt`, `pricing.completion`, `architecture.modality`. |
| **Mistral AI** | `GET /v1/models` | `Authorization: Bearer ...` | JSON array (OpenAI-compat format) | **ID pattern matching:** `mistral-*`, `ministral-*`, `pixtral-*`, `codestral-*` → Chat. `mistral-embed` → Embedding. |
| **xAI (Grok)** | `GET /v1/models` | `Authorization: Bearer xai-...` | JSON (OpenAI-compat) | **All Chat** — `grok-*` models. |
| **Venice AI** | `GET /v1/models` (OpenAI-compat) | `Authorization: Bearer ...` | JSON array (OpenAI-compat format) | **ID pattern matching + metadata:** Venice curates open-source models. Chat models: `llama-*`, `qwen-*`, `deepseek-*`, `dolphin-*`, `venice-*`. Image models: `flux-*`, `stable-diffusion-*`. Venice also offers audio (TTS/STT) via `/v1/audio/*`. Classify by model capability tags in response. Base URL: `https://api.venice.ai/api/v1`. |
| **Moonshot (Kimi)** | `GET /v1/models` (OpenAI-compat) | `Authorization: Bearer ...` | JSON array (OpenAI-compat format) | **All Chat** — Kimi K2/K2.5 models with long-context support (up to 256K tokens). Base URL: `https://api.moonshot.ai/v1` (international) or `https://api.moonshot.cn/v1` (China). |
| **MiniMax** | `GET /v1/models` (OpenAI-compat) | `Authorization: Bearer ...` | JSON array (OpenAI-compat format) | **All Chat** — MiniMax models (e.g., `minimax-m2.5`). Primarily text generation with some multimodal capabilities. |
| **NVIDIA NIM** | `GET /v1/models` | `Authorization: Bearer nvapi-...` | JSON array (OpenAI-compat format) | **All Chat** — NVIDIA-optimized models hosted via NIM containers. Models identified by `meta/llama-*`, `nvidia/*`, `mistralai/*` prefixes. Base URL: `https://integrate.api.nvidia.com/v1`. |
| **Baidu Qianfan** | Platform-specific API (not OpenAI-compat) | API key + Secret key | JSON with model list | **All Chat** — ERNIE series models (`ernie-x1-turbo-*`, `ernie-speed-*`). Uses Baidu-specific API format. Requires special endpoint handling. |
| **Xiaomi** | OpenAI-compat (limited) | `Authorization: Bearer ...` | JSON (OpenAI-compat format) | **All Chat** — MiMo VL and MiLM models. Newer provider with limited model selection; static registry recommended. |
| **Amazon Bedrock** | AWS SDK `ListFoundationModels` | AWS credentials (access key + secret key + region) | JSON array of `{ modelId, modelName, providerName, customizationsSupported, inferenceTypesSupported }` | **`providerName` + `modelId` pattern:** `anthropic.claude-*` → Chat. `amazon.titan-embed-*` → Embedding. `amazon.titan-image-*` → Diffusion. `stability.stable-diffusion-*` → Diffusion. `amazon.nova-*` → Chat. Bedrock aggregates 3rd-party models; classification is by model family. |
| **Together AI** | `GET /v1/models` | `Authorization: Bearer ...` | JSON array (OpenAI-compat + extra fields) | **`type` field:** `"chat"` → Chat. `"embedding"` → Embedding. `"image"` → Diffusion. |
| **Cohere** | `GET /v1/models` | `Authorization: Bearer ...` | JSON array of `{ name, endpoints[], ... }` | **`endpoints` field:** `"chat"` → Chat. `"embed"` → Embedding. `"rerank"` → Reranker (skip). |
| **ElevenLabs** | `GET /v1/models` + `GET /v1/voices` | `xi-api-key: ...` | Models: `{ model_id, name, languages[] }`. Voices: `{ voice_id, name, category }` | **All TTS** — ElevenLabs only offers TTS. Models define quality/speed trade-offs. Voices are listed separately and paired with models. |
| **Stability AI** | `GET /v1/engines/list` | `Authorization: Bearer sk-...` | JSON array of `{ id, name, description, type }` | **All Diffusion** — `type: "PICTURE"` filter. Models: `stable-diffusion-*`, `stable-image-*`. |
| **Deepgram** | No list API; known models | `Authorization: Token ...` | N/A (hardcode model IDs) | **All STT** — Deepgram models: `nova-3`, `nova-2`, `enhanced`, `base`. No list API; use static registry with version bump checks. |
| **Voyage AI** | No list API; known models | `Authorization: Bearer ...` | N/A | **All Embedding** — `voyage-3`, `voyage-3-large`, `voyage-code-3`. No list API; static registry. |
| **fal.ai** | No standard list; known endpoints | `Authorization: Key ...` | N/A | **All Diffusion** — Static model registry: `flux-dev`, `flux-schnell`, `sdxl`, `sd3-medium`. |

#### Architecture — `CloudModelRegistry`

```
backend/src/inference/
├── model_discovery/
│   ├── mod.rs              ← CloudModelRegistry struct (Tauri state), cache layer
│   ├── types.rs            ← CloudModelEntry, ModalityCategory enum, DiscoveryResult
│   ├── classifier.rs       ← classify_model(provider, model_id, metadata) → ModalityCategory
│   ├── openai.rs           ← discover_openai_models(api_key) → Vec<CloudModelEntry>
│   ├── anthropic.rs        ← discover_anthropic_models(api_key) → Vec<CloudModelEntry>
│   ├── gemini.rs           ← discover_gemini_models(api_key) → Vec<CloudModelEntry>
│   ├── groq.rs             ← discover_groq_models(api_key) → Vec<CloudModelEntry>
│   ├── openrouter.rs       ← discover_openrouter_models(api_key) → Vec<CloudModelEntry>
│   ├── mistral.rs          ← discover_mistral_models(api_key) → Vec<CloudModelEntry>
│   ├── xai.rs              ← discover_xai_models(api_key) → Vec<CloudModelEntry>
│   ├── venice.rs           ← discover_venice_models(api_key) → Vec<CloudModelEntry>
│   ├── moonshot.rs         ← discover_moonshot_models(api_key) → Vec<CloudModelEntry>
│   ├── nvidia.rs           ← discover_nvidia_models(api_key) → Vec<CloudModelEntry>
│   ├── bedrock.rs          ← discover_bedrock_models(credentials) → Vec<CloudModelEntry>
│   ├── together.rs         ← discover_together_models(api_key) → Vec<CloudModelEntry>
│   ├── cohere.rs           ← discover_cohere_models(api_key) → Vec<CloudModelEntry>
│   ├── elevenlabs.rs       ← discover_elevenlabs_models(api_key) → Vec<CloudModelEntry>
│   ├── stability.rs        ← discover_stability_models(api_key) → Vec<CloudModelEntry>
│   └── static_registry.rs  ← Deepgram, Voyage, fal.ai, MiniMax, Qianfan, Xiaomi (providers without list APIs or with non-standard APIs)
```

```rust
/// A single model discovered from a cloud provider
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct CloudModelEntry {
    pub id: String,                    // e.g. "gpt-5.1-chat-latest"
    pub display_name: String,          // e.g. "GPT-5.1 Chat"
    pub provider: String,              // e.g. "openai" — matches keychain::PROVIDERS slug
    pub provider_base_url: String,     // e.g. "https://api.openai.com/v1" — used by resolve_provider()
    pub modality: ModalityCategory,    // Chat | Embedding | TTS | STT | Diffusion
    pub context_window: Option<u32>,   // tokens (chat/embedding only)
    pub max_output_tokens: Option<u32>,
    pub pricing: Option<ModelPricing>, // cost per 1M input/output tokens
    pub supports_vision: bool,         // multimodal image input
    pub supports_tools: bool,          // function calling
    pub supports_streaming: bool,
    pub deprecated: bool,              // flagged but still listed
    pub created_at: Option<i64>,       // unix timestamp
    pub metadata: serde_json::Value,   // provider-specific extra fields
}

// NOTE: `provider_base_url` is populated from b2-9's PROVIDER_ENDPOINTS constant.
// When user selects a discovered model, the frontend sends (provider, model_id)
// back to the backend. `resolve_provider()` looks up base_url from either
// the CloudModelEntry or PROVIDER_ENDPOINTS. This eliminates hardcoded URLs
// in the match arms.

#[derive(Serialize, Deserialize, Clone, Type)]
pub enum ModalityCategory {
    Chat,
    Embedding,
    TTS,
    STT,
    Diffusion,
}

#[derive(Serialize, Deserialize, Clone, Type)]
pub struct ModelPricing {
    pub input_per_million: Option<f64>,   // $ per 1M input tokens
    pub output_per_million: Option<f64>,  // $ per 1M output tokens
    pub per_image: Option<f64>,           // $ per image (diffusion)
    pub per_minute: Option<f64>,          // $ per minute (STT)
    pub per_char: Option<f64>,            // $ per 1K characters (TTS)
}

/// Cache layer — avoids hammering provider APIs on every UI render
pub struct CloudModelRegistry {
    cache: Mutex<HashMap<String, CachedDiscovery>>,  // key = provider_id
    ttl: Duration,  // default: 30 minutes
}

struct CachedDiscovery {
    models: Vec<CloudModelEntry>,
    fetched_at: Instant,
}
```

#### Discovery Flow (triggered by frontend)

```
User opens Models tab in Settings
  │
  ├── Frontend calls `discover_cloud_models(providers: Vec<String>)` Tauri command
  │     providers = list of provider IDs where API key is set + granted
  │     (e.g. ["openai", "anthropic", "gemini", "groq"])
  │
  ├── Backend: CloudModelRegistry.discover(providers)
  │     For each provider (parallel, tokio::join!):
  │       ├── Check cache: if valid (< 30 min old) → return cached
  │       ├── Else: call provider-specific discover_*_models(api_key)
  │       │     ├── GET /v1/models (or provider-specific endpoint)
  │       │     ├── Parse response JSON
  │       │     ├── For each model:
  │       │     │     ├── classifier::classify_model(provider, id, metadata) → ModalityCategory
  │       │     │     ├── Extract context_window, pricing, capabilities
  │       │     │     ├── Filter out: fine-tuned, deprecated, internal models
  │       │     │     └── Build CloudModelEntry
  │       │     └── Return Vec<CloudModelEntry>
  │       └── Update cache with TTL
  │
  ├── Return: Vec<CloudModelEntry> (all providers merged)
  │
  └── Frontend: ModelBrowser renders cloud models alongside local models
        ├── Source filter: All / 🖥️ Local / ☁️ Cloud
        ├── Category filter: Chat / Embedding / TTS / STT / Diffusion / All
        └── Cloud models grouped by provider, show: name, modality badge,
            context window, pricing, "Select" button
```

#### Model Classification Logic (`classifier.rs`)

```rust
pub fn classify_model(
    provider: &str,
    model_id: &str,
    metadata: &serde_json::Value,
) -> Option<ModalityCategory> {
    match provider {
        "openai" => {
            if model_id.starts_with("gpt-") || model_id.starts_with("o1-")
               || model_id.starts_with("o3-") || model_id.starts_with("o4-")
               || model_id.starts_with("chatgpt-") {
                Some(ModalityCategory::Chat)
            } else if model_id.starts_with("text-embedding-") {
                Some(ModalityCategory::Embedding)
            } else if model_id.starts_with("tts-") {
                Some(ModalityCategory::TTS)
            } else if model_id.starts_with("whisper-") {
                Some(ModalityCategory::STT)
            } else if model_id.starts_with("dall-e-") {
                Some(ModalityCategory::Diffusion)
            } else {
                None  // skip unknown models (fine-tunes, deprecated, etc.)
            }
        }
        "anthropic" => Some(ModalityCategory::Chat),
        "gemini" => {
            // Use supportedGenerationMethods from API response
            let methods = metadata.get("supportedGenerationMethods")
                .and_then(|v| v.as_array());
            if let Some(m) = methods {
                let methods_str: Vec<&str> = m.iter().filter_map(|v| v.as_str()).collect();
                if methods_str.contains(&"embedContent") {
                    Some(ModalityCategory::Embedding)
                } else if methods_str.contains(&"generateContent") {
                    Some(ModalityCategory::Chat)
                } else {
                    None
                }
            } else {
                None
            }
        }
        "groq" => {
            if model_id.starts_with("whisper-") {
                Some(ModalityCategory::STT)
            } else {
                Some(ModalityCategory::Chat)
            }
        }
        "cohere" => {
            let endpoints = metadata.get("endpoints")
                .and_then(|v| v.as_array());
            if let Some(eps) = endpoints {
                let eps_str: Vec<&str> = eps.iter().filter_map(|v| v.as_str()).collect();
                if eps_str.contains(&"embed") {
                    Some(ModalityCategory::Embedding)
                } else if eps_str.contains(&"chat") {
                    Some(ModalityCategory::Chat)
                } else {
                    None
                }
            } else {
                None
            }
        }
        "together" => {
            match metadata.get("type").and_then(|v| v.as_str()) {
                Some("chat") => Some(ModalityCategory::Chat),
                Some("embedding") => Some(ModalityCategory::Embedding),
                Some("image") => Some(ModalityCategory::Diffusion),
                _ => None,
            }
        }
        "elevenlabs" => Some(ModalityCategory::TTS),
        "stability" => Some(ModalityCategory::Diffusion),
        "openrouter" => Some(ModalityCategory::Chat),
        "mistral" => {
            if model_id.contains("embed") {
                Some(ModalityCategory::Embedding)
            } else {
                Some(ModalityCategory::Chat)
            }
        }
        "xai" => Some(ModalityCategory::Chat),
        "venice" => {
            // Venice hosts open-source models across modalities
            if model_id.starts_with("flux-") || model_id.starts_with("stable-diffusion-")
               || model_id.contains("sdxl") {
                Some(ModalityCategory::Diffusion)
            } else {
                Some(ModalityCategory::Chat)  // Default: most Venice models are chat
            }
        }
        "moonshot" => Some(ModalityCategory::Chat),   // Kimi K2/K2.5 — all chat
        "minimax" => Some(ModalityCategory::Chat),    // MiniMax M2.5 — all chat
        "nvidia" => Some(ModalityCategory::Chat),     // NVIDIA NIM — all chat
        "qianfan" => Some(ModalityCategory::Chat),    // ERNIE — all chat
        "xiaomi" => Some(ModalityCategory::Chat),     // MiMo VL — all chat
        "bedrock" | "amazon-bedrock" => {
            // Bedrock aggregates models from multiple providers
            if model_id.contains("titan-embed") {
                Some(ModalityCategory::Embedding)
            } else if model_id.contains("titan-image") || model_id.contains("stable-diffusion") {
                Some(ModalityCategory::Diffusion)
            } else {
                Some(ModalityCategory::Chat)  // claude, nova, llama — all chat
            }
        }
        _ => None,
    }
}
```

#### Tasks

| # | Task | Files | Priority |
|---|------|-------|----------|
| B9-1 | Create `backend/src/inference/model_discovery/` module tree: `mod.rs`, `types.rs`, `classifier.rs` | `inference/model_discovery/` | **P0** |
| B9-2 | Define `CloudModelEntry`, `ModalityCategory`, `ModelPricing` types with `specta` + `serde` derives for auto-generated TS bindings | `model_discovery/types.rs` | **P0** |
| B9-3 | Implement `CloudModelRegistry` with `Mutex<HashMap<String, CachedDiscovery>>` cache (30-min TTL default); register as Tauri managed state | `model_discovery/mod.rs`, `lib.rs` | **P0** |
| B9-4 | Implement `classify_model()` — provider-aware model ID → `ModalityCategory` classification with pattern matching and metadata inspection (covering all 21 providers) | `model_discovery/classifier.rs` | **P0** |
| | **— Primary Providers (dedicated key commands) —** | | |
| B9-5 | Implement OpenAI model discovery: `GET /v1/models`, filter by `owned_by` (skip `system`, `openai-internal`), classify by ID prefix | `model_discovery/openai.rs` | **P0** |
| B9-6 | Implement Anthropic model discovery: `GET /v1/models` with `anthropic-version` header, all → Chat | `model_discovery/anthropic.rs` | **P0** |
| B9-7 | Implement Gemini model discovery: `GET /v1beta/models?key=...`, classify by `supportedGenerationMethods` field | `model_discovery/gemini.rs` | **P0** |
| B9-8 | Implement Groq model discovery: `GET /openai/v1/models`, classify `whisper-*` → STT, rest → Chat | `model_discovery/groq.rs` | **P1** |
| B9-9 | Implement OpenRouter model discovery: `GET /api/v1/models`, extract rich pricing + context_length metadata, all → Chat | `model_discovery/openrouter.rs` | **P1** |
| | **— Implicit Providers (via `save_implicit_provider_key`) —** | | |
| B9-10 | Implement Mistral AI model discovery: `GET /v1/models` at `https://api.mistral.ai/v1`, classify `mistral-embed` → Embedding, rest → Chat | `model_discovery/mistral.rs` | **P1** |
| B9-11 | Implement xAI (Grok) model discovery: `GET /v1/models` at `https://api.x.ai/v1`, all → Chat. `grok-*` models. | `model_discovery/xai.rs` | **P1** |
| B9-12 | Implement Venice AI model discovery: `GET /v1/models` at `https://api.venice.ai/api/v1` (OpenAI-compat). Classify: `flux-*`/`stable-diffusion-*` → Diffusion, rest → Chat. Venice curates rotating open-source models. | `model_discovery/venice.rs` | **P1** |
| B9-13 | Implement Together AI model discovery: `GET /v1/models` at `https://api.together.xyz/v1`, classify by `type` field → Chat / Embedding / Diffusion | `model_discovery/together.rs` | **P1** |
| B9-14 | Implement Moonshot (Kimi) model discovery: `GET /v1/models` at `https://api.moonshot.ai/v1` (OpenAI-compat), all → Chat. Kimi K2/K2.5 long-context models. | `model_discovery/moonshot.rs` | **P2** |
| B9-15 | Implement NVIDIA NIM model discovery: `GET /v1/models` at `https://integrate.api.nvidia.com/v1`, all → Chat. Enterprise-grade NVIDIA-optimized models. | `model_discovery/nvidia.rs` | **P2** |
| B9-16 | Implement Amazon Bedrock model discovery: use AWS SDK `ListFoundationModels` API with regional endpoint. Classify: `*titan-embed*` → Embedding, `*titan-image*`/`*stable-diffusion*` → Diffusion, rest → Chat. Requires AWS credentials (access key + secret + region). | `model_discovery/bedrock.rs` | **P2** |
| | **— TTS/STT/Diffusion-Specific Providers —** | | |
| B9-17 | Implement ElevenLabs model + voice discovery: `GET /v1/models` + `GET /v1/voices`, all → TTS, pair voice list with each model | `model_discovery/elevenlabs.rs` | **P1** |
| B9-18 | Implement Stability AI model discovery: `GET /v1/engines/list`, filter `type: "PICTURE"`, all → Diffusion | `model_discovery/stability.rs` | **P1** |
| B9-19 | Implement Cohere model discovery: `GET /v1/models`, classify by `endpoints[]` → Chat or Embedding | `model_discovery/cohere.rs` | **P2** |
| | **— Static Registries (no list API available) —** | | |
| B9-20 | Implement static registries for providers without list APIs or with non-standard APIs: Deepgram (STT: `nova-3`, `nova-2`), Voyage AI (Embedding: `voyage-3`, `voyage-3-large`, `voyage-code-3`), fal.ai (Diffusion: `flux-dev`, `flux-schnell`, `sdxl`), MiniMax (Chat: `minimax-m2.5`), Baidu Qianfan (Chat: `ernie-x1-turbo-32k`), Xiaomi (Chat: `mimo-vl-7b`) — hardcoded but versioned, easy to update | `model_discovery/static_registry.rs` | **P1** |
| | **— Tauri Commands & Frontend Integration —** | | |
| B9-21 | Register `discover_cloud_models(providers: Vec<String>)` Tauri command — takes `State<'_, SecretStore>` to read API keys for each requested provider. Provider IDs match `keychain::PROVIDERS` slugs: `openai`, `anthropic`, `gemini`, `groq`, `openrouter`, `xai`, `venice`, `together`, `moonshot`, `minimax`, `nvidia`, `qianfan`, `mistral`, `xiaomi`, `bedrock`, `elevenlabs`, `stability`, `cohere`, `deepgram`, `voyage`, `fal`. For each provider, calls `secret_store.get(provider_id)` → if `Some(key)`, invokes the discovery module; if `None`, skips. Returns flat `Vec<CloudModelEntry>`. | `inference/model_discovery/mod.rs`, `lib.rs` | **P0** |
| B9-22 | Register `refresh_cloud_models(provider: String)` Tauri command — force-invalidates cache for a single provider and re-fetches. Key from `SecretStore`. | `inference/model_discovery/mod.rs`, `lib.rs` | **P1** |
| B9-23 | Add `cloud_model_cache_ttl_minutes` to `UserConfig` (default: 30) — configurable TTL for discovery cache | `backend/src/config.rs` | **P2** |
| B9-24 | Frontend: create `useCloudModels` hook — calls `discover_cloud_models` on mount, passing provider IDs where a key exists. Provider IDs derived from `OpenClawStatus` `has_*_key` flags (which reflect `keychain::get_key()` presence). Caches in React state, exposes `models`, `loading`, `refreshProvider()`. | `frontend/src/hooks/use-cloud-models.ts` | **P0** |
| B9-25 | Frontend: integrate `useCloudModels` into `ModelBrowser.tsx` — merge cloud models with local `MODEL_LIBRARY` + HF discovery results based on source filter state | `frontend/src/components/settings/ModelBrowser.tsx` | **P0** |
| B9-26 | Frontend: integrate `useCloudModels` into `ModelSelector.tsx` (chat view model picker) — cloud chat models appear alongside local models, grouped by provider | `frontend/src/components/chat/ModelSelector.tsx` | **P0** |
| B9-27 | Deprecation/removal: reduce `model-library.ts` to **local models only** (GGUF, Diffusion, Embedding, STT, TTS that require download). Remove all hardcoded cloud model entries — these now come from live discovery. Keep as fallback for offline use. | `frontend/src/lib/model-library.ts` | **P1** |
| B9-28 | Error handling: if provider API is unreachable (key expired, rate limited, network error), fall back to last cached result + show "⚠️ Could not refresh models" badge per provider | `model_discovery/mod.rs`, `ModelBrowser.tsx` | **P1** |
| B9-29 | Generate `specta` TypeScript bindings for `CloudModelEntry`, `ModalityCategory`, `ModelPricing`, and all new Tauri commands; regenerate `bindings.ts` | `backend/src/lib.rs`, `frontend/src/lib/bindings.ts` | **P0** |
| B9-30 | **Context size propagation** — when a user selects a discovered cloud model, store the model's `context_window` alongside the model ID in `UserConfig` (new field: `selected_model_context_size: Option<u32>`). `resolve_provider()` reads this instead of hardcoding `context_size: 200000` for Anthropic, `128000` for OpenAI, etc. This is critical for providers like Moonshot (256K), MiniMax (200K), Venice (32K–128K depending on model), and NVIDIA (varies per hosted model). If no discovered metadata is available, fall back to `PROVIDER_ENDPOINTS.default_context_size`. | `backend/src/config.rs`, `backend/src/chat.rs`, `frontend/src/hooks/use-cloud-models.ts` | **P0** |

#### Current Codebase Provider Key Map (all 21 providers)

This table maps every provider that currently has API key support in the codebase to its Keychain identifier, key command type, API compatibility, and model discovery strategy.

| Provider | Keychain / Provider ID | Key Command | API Compat | Model Discovery |
|----------|----------------------|-------------|------------|----------------|
| Anthropic | `anthropic` | Dedicated (`openclaw_save_anthropic_key`) | Anthropic API | Live: `GET /v1/models` |
| OpenAI | `openai` | Dedicated (`openclaw_save_openai_key`) | OpenAI API | Live: `GET /v1/models` |
| Google Gemini | `gemini` | Dedicated (`openclaw_save_gemini_key`) | Gemini REST | Live: `GET /v1beta/models` |
| Groq | `groq` | Dedicated (`openclaw_save_groq_key`) | OpenAI-compat | Live: `GET /openai/v1/models` |
| OpenRouter | `openrouter` | Dedicated (`openclaw_save_openrouter_key`) | OpenAI-compat | Live: `GET /api/v1/models` |
| xAI (Grok) | `xai` | Implicit (`openclaw_save_implicit_provider_key`) | OpenAI-compat | Live: `GET /v1/models` |
| Venice AI | `venice` | Implicit | OpenAI-compat | Live: `GET /v1/models` |
| Together AI | `together` | Implicit | OpenAI-compat | Live: `GET /v1/models` |
| Moonshot (Kimi) | `moonshot` | Implicit | OpenAI-compat | Live: `GET /v1/models` |
| MiniMax | `minimax` | Implicit | OpenAI-compat | Static registry |
| NVIDIA NIM | `nvidia` | Implicit | OpenAI-compat | Live: `GET /v1/models` |
| Baidu Qianfan | `qianfan` | Implicit | Baidu API | Static registry |
| Mistral AI | `mistral` | Implicit | OpenAI-compat | Live: `GET /v1/models` |
| Xiaomi | `xiaomi` | Implicit | OpenAI-compat | Static registry |
| Amazon Bedrock | `bedrock` | Special (`openclaw_save_bedrock_credentials`) | AWS SDK | Live: `ListFoundationModels` |
| Brave Search | `brave` | Dedicated (`openclaw_save_brave_key`) | Brave API | N/A (not inference) |
| Hugging Face | `huggingface` | Dedicated (`openclaw_set_hf_token`) | HF API | N/A (used for downloads) |
| ElevenLabs | *(future)* | *(B8-1)* | ElevenLabs API | Live: `GET /v1/models` + `/v1/voices` |
| Stability AI | *(future)* | *(B8-1)* | Stability API | Live: `GET /v1/engines/list` |
| Deepgram | *(future)* | *(B8-1)* | Deepgram API | Static registry |
| Voyage AI | *(future)* | *(B8-1)* | Voyage API | Static registry |

#### Interaction with Existing Systems

| System | Before | After |
|--------|--------|-------|
| `model-library.ts` (46 KB) | Hardcodes ALL models (local + cloud). 1114 lines, mostly cloud model entries with URLs that go stale. | **Local models only** — GGUF downloads, HF links, diffusion weights. < 500 lines. Cloud models come from live API. |
| `ModelBrowser.tsx` | Shows: Curated tab (from `model-library.ts`) + Discover tab (HF Hub search). No cloud provider models. | Shows: **Local** tab (curated + HF discovery) + **Cloud** tab (live from `CloudModelRegistry`) + **All** (merged) with category sub-filters. |
| `ModelSelector.tsx` (chat) | Hardcoded provider/model dropdowns from `model-library.ts` for cloud, manual entry for local. | Cloud models auto-populated from `useCloudModels`. User sees latest models immediately when provider releases them. |
| `HFDiscovery.tsx` | Searches HF Hub for local-downloadable models (GGUF, MLX, etc.). Unrelated to cloud providers. | **Unchanged** — HF discovery is for downloadable local models only. Cloud discovery is separate. |
| `InferenceModeTab.tsx` | (new) Shows backend picker per modality. | Model selector within each modality section uses cloud-discovered models for the active provider. |

---

## Dependency Map

```
Work Stream A (Cloud Storage):
  A1 (Foundation) ──► A2 (Migration Engine) ──► A5 (Frontend)
  A1 ──────────────► A3 (FileStore) ──────────► A5
  A2 + A3 ─────────► A4 (Additional Providers)
  A2 + A4 + A5 ────► A6 (Tests)

Work Stream B (Inference Separation):
  SecretStore (existing) ──► B1 (InferenceRouter reads keys from SecretStore)
  B1 (InferenceRouter) ──► B2 (Chat)
  B1 ──────────────────► B3 (Embedding)  ──────► B7 (Frontend + Models UI)
  B1 ──────────────────► B4 (TTS)        ──────► B7
  B1 ──────────────────► B5 (STT)        ──────► B7
  B1 ──────────────────► B6 (Diffusion)  ──────► B7
  B2+B3+B4+B5+B6 ──────► B8 (Cross-modal: Secrets, Config, bindings)
  SecretStore ──────────► B9 (Model Discovery — reads keys from SecretStore to call list APIs)
  B9 ───────────────────► B7 (Frontend Models UI consumes discovered models)

Key architecture flow:
  Keychain (macOS) → keychain.rs (single cache) → SecretStore (app-level)
                                                     ├── InferenceRouter (B1)
                                                     ├── CloudModelRegistry (B9)
                                                     └── OpenClawConfig (engine only, NOT used by inference)

Cross-stream:
  A3 (FileStore) ◄──── required by B6 (diffusion cloud output stored via FileStore in cloud mode)
  B3 (Embedding) ◄──── affects A (vector index on cloud restore requires embedding backend to be set)
  B9 (Discovery) ◄──── independent of A; can be implemented in parallel
```

---

## Open Decisions

| # | Question | Options | Recommendation |
|---|----------|---------|----------------|
| 1 | **Default embedding backend** | Local (llama-server) / Cloud (OpenAI) | Local for offline-first; offer cloud in wizard |
| 2 | **Embedding dimension change UX** | Silent re-index / Warn + confirm / Block until rebuilt | Warn + confirm: show estimated rebuild time |
| 3 | **TTS audio format on frontend** | Always convert to WAV / Handle MP3 natively via `<audio>` / Decode in AudioContext | Web Audio API decoding handles both PCM and MP3 via `decodeAudioData` |
| 4 | **Cloud-only mode** | Fully suppress local sidecar startup / Keep optional | Suppress only when `chat_backend != "local"` — avoids wasteful CPU |
| 5 | **OpenClaw + cloud inference backends** | Separate codepaths / Share `InferenceRouter` | OpenClaw manages its own providers via openclaw-engine; don't force unification |
| 6 | **fal.ai polling timeout** | 30s / 120s / User-configurable | 120s (FLUX models vary widely in server queue time) |
| 7 | **Embedding cost control** | No limit / Per-request estimate / Monthly cap | Per-request estimate in UI; no hard limit in v1 |
| 8 | **Cloud storage + cloud inference interaction** | Independent settings / Bundle as "Full Cloud Mode" | Keep independent — separate concerns, different keys |
| 9 | **Model discovery cache TTL** | 10 min / 30 min / 60 min / No cache | 30 min — frequent enough to catch new model releases, infrequent enough to avoid rate limits |
| 10 | **Discovery trigger** | On every Models tab open / On first open per session / Manual refresh only | On first open + manual refresh button per provider; re-discover on key change |
| 11 | **Offline model list fallback** | Show empty / Show last cached / Show hardcoded `model-library.ts` entries | Last cached + hardcoded fallback. Mark as "⏱ Last updated: X ago" |
| 12 | **Model discovery for providers without list APIs** (Deepgram, Voyage, fal.ai) | Hardcode and update per release / Scrape docs / Skip discovery | Hardcode in `static_registry.rs`, version-tagged, easy to update. Re-check quarterly. |
| 13 | **model-library.ts migration path** | Delete all cloud entries immediately / Deprecate gradually / Keep as offline fallback | Keep as offline fallback — if no API key is set, user still sees cloud model names (grayed out) to understand what's available |
| 14 | **`selected_cloud_model` ownership** | Keep on `OpenClawIdentity` (agent concern) / Move to `UserConfig` (app concern) / Both | Keep on `OpenClawIdentity` for the OpenClaw engine's model selection and add a separate `UserConfig.chat_model` for the Rig chat path. The two can differ: the agent's model is for autonomous tasks, the chat model is for the user's chat UI. `CloudBrainConfigModal` writes to `OpenClawIdentity.selected_cloud_model`; the new ModelSelector writes to `UserConfig.chat_model`. |
| 15 | **Image generation tool backend** | Always use active `diffusion_backend` / Prefer local for speed / Let user configure separately | Always use active `diffusion_backend` from `InferenceRouter` — consistency is more important than micro-optimizing. If user set cloud diffusion, the chat tool should use it too. |
| 16 | **`fal` Keychain slug** | `fal` / `fal_ai` / `fal.ai` | Use `fal` — consistent with other slugs (lowercase, no dots or underscores), matches `keychain::PROVIDERS` convention. Frontend display name is "fal.ai" but the slug is `fal`. |
