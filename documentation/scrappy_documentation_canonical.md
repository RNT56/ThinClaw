# Scrappy – Canonical Technical Documentation

> **Last updated:** 2026-03-13
> **Status:** Current — reflects the live codebase

This document is the **single source of truth** for Scrappy's architecture, implementation, and operational decisions. It supersedes all prior conflicting instructions.

Scrappy is a **personal, local-first desktop application** built with **Tauri v2 (frontend)**, **Rust (backend)**, and a **multi-engine local inference system**. Its agentic capabilities are powered by **IronClaw**, an in-process Rust library.

---

## 1. Canonical Architecture Overview

### Process Model

```
Frontend (React / TypeScript in WebView)
   ↓ invoke() — via tauri-specta type-safe bindings
Rust Backend (Tauri Commands + Managed State)
   ├── SidecarManager → llama-server (Chat / Embed / Summarize)
   ├── SidecarManager → whisper-server (STT)
   ├── EngineManager  → InferenceEngine trait (LlamaCpp / MLX / vLLM / Ollama)
   ├── InferenceRouter → 5-modality routing (Chat, Embedding, TTS, STT, Diffusion)
   ├── IronClawState  → ironclaw crate (in-process agent engine, NO sidecar)
   ├── Rig Agent       → Native Rust agentic pipeline (orchestrator, tools, RAG)
   └── FileStore       → Local-first + cloud-ready I/O abstraction
```

**Hard rules:**
> - The frontend NEVER talks directly to any sidecar or inference server.
> - All requests go through Rust Tauri commands. This guarantees security (tokens never exposed), consistent request shaping, and no port conflicts.
> - IronClaw runs **in-process** as a Rust library crate — there is no Node.js sidecar, no WebSocket bridge.

### Dual-Agent Architecture

| Concern | Chat Tab (Rig Agent) | OpenClaw Tab (IronClaw) |
|---|---|---|
| Entry point | `chat_stream` Tauri command | `openclaw_send_message` Tauri command |
| Agent loop | `Orchestrator::run_turn()` | IronClaw `Agent` background task |
| Streaming | `tauri::Channel<StreamChunk>` | `TauriChannel` → `app.emit("openclaw-event")` |
| Tools | Rig tool registry + MCP sandbox | IronClaw `ToolRegistry` + `SafetyLayer` |
| Database | Scrappy's SQLite (sqlx) — `openclaw.db` | IronClaw's libSQL (separate file) |

---

## 2. Sidecar Lifecycle & Security

### Port & Token Policy

- Bind address: `127.0.0.1`
- Port: **OS-assigned free port** via `TcpListener::bind("127.0.0.1:0")`
- Auth: **random per-launch API token** (32-char alphanumeric)
- Storage: in-memory only (never persisted to disk)

### Spawn Flow

1. `SidecarManager` selects free port.
2. Generates random 32-character API token.
3. Spawns sidecar (`llama-server`, `whisper-server`, etc.) with host, port, and token.
4. Tracks PID via `ProcessTracker` (persisted to JSON for orphan cleanup on restart).
5. Polls `/health` every 500ms with 120s timeout until ready.
6. Emits `sidecar_event` for frontend progress tracking.
7. Marks model "ready" in UI.

### Smart Discovery (Vision)
The manager automatically detects vision projectors (`.mmproj`) by scanning the model's subfolder for files matching the GGUF name or containing "mmproj" flags, ensuring zero-config multimodal support.

### Failure Handling

- `ProcessTracker` persists PIDs to JSON — `cleanup_all()` kills orphans on app restart.
- Crash detection via `sidecar_event` emissions.
- `Drop` impl on `SidecarProcess` for automatic cleanup on struct drop.

---

## 3. Multi-Engine Inference System

### Architecture

Each build ships with **one primary inference engine** selected via Cargo feature flags. There is no runtime engine switching.

```toml
# backend/Cargo.toml
[features]
default = ["llamacpp"]
llamacpp = []    # Bundled binary sidecar — all platforms
mlx = []         # External Python process — macOS Apple Silicon only
vllm = []        # External Python process — Linux CUDA only
ollama = []      # Detect existing daemon — all platforms
```

### Engine × Platform Matrix

| Engine | macOS ARM | macOS Intel | Linux CUDA | Windows |
|---|---|---|---|---|
| **llama.cpp** | ✅ Metal | ✅ CPU | ✅ CUDA | ✅ |
| **MLX** | ✅ Best option | ❌ | ❌ | ❌ |
| **vLLM** | ❌ | ❌ | ✅ Best CUDA | ❌ |
| **Ollama** | ✅ | ✅ | ✅ | ✅ |

### InferenceEngine Trait

All engines implement `InferenceEngine` (in `backend/src/engine/`):
- `start(model_path, context_size) → Result<u16>`
- `stop() → Result<()>`
- `is_ready() → bool`
- `base_url() → String`
- `display_name() → &'static str`

### InferenceRouter

The `InferenceRouter` (in `backend/src/inference/`) routes requests across **5 modalities**:
- **Chat** — local sidecar or cloud providers
- **Embedding** — local embedding server or cloud APIs (OpenAI, Gemini, Cohere, Voyage)
- **TTS** — Piper sidecar or cloud (ElevenLabs, OpenAI, Gemini)
- **STT** — Whisper sidecar or cloud (Deepgram, OpenAI, Gemini)
- **Diffusion** — sd.cpp CLI or cloud (DALL·E, fal.ai, Imagen, Stability, Together)

All local engines expose an **OpenAI-compatible HTTP API** on localhost, so the `rig_lib` stack is engine-agnostic.

---

## 4. Streaming UX

### Rendering Strategy

- Token buffering with DOM update cadence of ~30–60ms or sentence boundary
- Virtualized message list
- Always-visible Stop button
- No UI reflow during streaming

### Message States

- Queued (waiting for slot)
- Running (streaming)
- Cancelled
- Complete

---

## 5. Data Layer

### SQLite as Single Source of Truth

Database file: `openclaw.db` (legacy `scrappy.db` auto-migrated).

Tables (13 migration files through `20260301000000_cloud_storage.sql`):
- **projects** — logical partitioning of work
- **conversations** — with `project_id` FK
- **messages** — with `conversation_id` FK
- **documents** / **chunks** — RAG pipeline
- **generated_images** — Imagine Studio history
- **user_settings** — persisted user preferences
- **cloud_storage** — cloud sync metadata

### Timestamp Convention
All timestamps normalized to **milliseconds (Unix epoch)** via migration `20260225000000_normalize_timestamps.sql`.

### Partitioning Strategy
- Conversations and RAG documents are scoped to a **Project**.
- Deleting a project triggers cascading cleanup of associated transcripts, chunks, and vector indices.

### Full-Text Search

- SQLite FTS5 with BM25 ranking
- Used for lexical retrieval in hybrid search

---

## 6. Vector Storage

### Technology: `usearch`

- High-performance vector search engine via the `usearch` Rust crate.
- Managed as external `.usearch` index files alongside SQLite.
- Document chunks stored in SQLite with IDs corresponding to vector indices.
- Integrity check tool verifies vector index vs. SQLite chunk alignment.

---

## 7. RAG Pipeline

### Ingestion

1. Extract text (PDF/MD/TXT) — OCR fallback via Chromium for PDFs
2. SHA256 content deduplication
3. Chunk (300–800 tokens)
4. Generate embedding (via InferenceRouter — local sidecar or cloud)
5. Store text + embedding in SQLite + USearch index

### Retrieval

1. Lexical search (FTS5 BM25)
2. Vector search (`usearch` cosine similarity)
3. Merge + dedupe
4. Rerank (ONNX cross-encoder)
5. Token-budgeted context pack
6. Generate answer with source IDs

---

## 8. Reranking

### Implementation

- **In-process** via ONNX Runtime (`ort` crate)
- `RerankerWrapper` handles initialization and optional model download
- Model: `cross-encoder/ms-marco-MiniLM-L-6-v2` (~20–30MB ONNX)

### Execution

- Load once at startup
- Rerank top 10–30 candidates
- CPU inference (<100ms typical)

---

## 9. Embedding Cache Policy

- Chunk embeddings cached by content hash (permanent)
- Query embeddings cached with TTL (5–30 min)
- LRU eviction
- Invalidate cache on embedding model change

---

## 10. Multimodal Vision Pipeline

### Supported Models

- mmproj-based models (Pixtral, LLaVA, etc.)

### Workflow

1. User drags/pastes image
2. Frontend shows preview
3. Rust backend stores image in app data, resizes/compresses, base64 encodes
4. Injected into request payload with `image_data` array

**Rule:** Frontend never handles base64 — Rust owns conversion & memory.

---

## 11. Imagine Studio

### Provider Hybrid Model
- **Local Diffusion**: Powered by `sd.cpp` CLI invocation. Supports resolution, VAE/CLIP component resolution, and sampling progress.
- **Cloud Generation**: Google Gemini Imagen 3, fal.ai, Stability AI — routed via `InferenceRouter`.

### Key Features
- **Style Library**: 45 presets in `style-library.ts`
- **Resolution Control**: User-selectable outputs (512px, 1024px, 2048px)
- **Integrated Gallery**: Persistent storage with favoriting, search, and **settings restoration** (one-click restore of prompt/styles)
- **Local/Cloud Badges**: UI indicates which backend is active per provider

---

## 12. Agent Engine (IronClaw — In-Process Rust Library)

### Architecture

IronClaw is linked as a **Rust crate dependency** — not a separate process:

```toml
# backend/Cargo.toml
ironclaw = { path = "../ironclaw", features = ["desktop"] }
```

All 66+ `openclaw_*` Tauri commands call `ironclaw::api::*` functions **directly** — no WebSocket, no Node.js, no serialization overhead.

### Key Bridge Files

| File | Purpose |
|---|---|
| `backend/src/openclaw/ironclaw_bridge.rs` | `IronClawState` lifecycle — init, config, Agent construction, shutdown |
| `backend/src/openclaw/ironclaw_channel.rs` | `TauriChannel` implementing `ironclaw::channels::Channel` trait |
| `backend/src/openclaw/ironclaw_types.rs` | `StatusUpdate` → `UiEvent` conversion |
| `backend/src/openclaw/ironclaw_secrets.rs` | `KeychainSecretsAdapter` bridging macOS Keychain to IronClaw |
| `backend/src/openclaw/tool_bridge.rs` | MCP tool bridge — routes IronClaw tool calls to `scrappy-mcp-tools` |
| `backend/src/openclaw/remote_proxy.rs` | Remote gateway proxy for fleet/remote agent connections |
| `backend/src/openclaw/ui_types.rs` | `UiEvent` enum + `UiSession`, `UiMessage`, `UiUsage` |
| `backend/src/openclaw/sanitizer.rs` | LLM token stripping (ChatML, Llama, Jinja markers) |

### Secrets Firewall

- API keys stored in macOS Keychain via `SecretStore`.
- Keys bridged to IronClaw via `KeychainSecretsAdapter` implementing `ironclaw::secrets::SecretsStore` trait.
- **Explicit Grant**: Users must toggle "Grant Access" per secret. A key existing in the Keychain does NOT mean IronClaw can use it.

### Workspace Isolation

- Each agent operates in a dedicated workspace directory (`OpenClaw/workspace`).
- Transcripts and artifacts are isolated from the main chat history.

### Subagent Systems

Two sub-agent systems coexist:
1. **System A (In-Process Executor)**: Lightweight ephemeral sub-tasks via `SubagentExecutor` — limited context, no persistence.
2. **System B (Session-Based)**: Full persistent chat sessions via `openclaw_spawn_session` — own memory, workspace access, managed in Fleet panel.

---

## 13. Intelligent Search Grounding

### Logic
Scrappy monitors user queries for factual indicators (dates, prices, entity questions).

### Behavior
- **Auto-Trigger**: Routes queries through `DDGSearchTool` + Chromium scraper even without explicit request.
- **Exception handling**: Common greetings and abstract queries bypass grounding.

---

## 14. Model Management & HF Hub Discovery

- **GGUF Metadata**: v2/v3 support — extracts architecture, context length, chat template, family detection (8 families)
- **Model Download**: `reqwest` streaming + resume (range requests) + cancellation + HF token auth
- **HF Hub Discovery**: Live search via HF API, file-tree parsing, smart multi-file downloads
- **Standard Assets**: Pre-defined auto-download (embedding model, reranker)
- **Stop Token Mapping**: 7 family-specific stop token sets

---

## 15. Cloud Storage

7 cloud storage providers implemented in `backend/src/cloud/providers/`, plus a local-only default mode:
- Amazon S3
- Google Drive
- iCloud
- OneDrive
- Dropbox
- WebDAV
- SFTP

All modes (including **Local**, the default) are abstracted behind `FileStore` for local-first, cloud-ready I/O.

---

## 16. Type Safety

### Implementation

- `tauri-specta` generates type-safe TypeScript bindings from Rust structs
- Bindings exported to `frontend/src/lib/bindings.ts`
- Frontend build fails on type mismatch
- All Rust command parameter/return types annotated with `#[specta::specta]`

---

## 17. Updates Strategy

- `tauri-plugin-updater` integrated and active
- Update check on app launch
- GitHub Releases as distribution channel

---

## 18. Logging & Debug

### Logging

- `tracing` + `tracing-appender` rolling log files
- Retention: last 7 days

### Debug Bundle

Includes: version + OS, profiles, logs, RAG stats
Excludes: model weights, raw chat (optional redact)

---

## 19. Global Shortcuts

| Shortcut | Action |
|---|---|
| `Cmd+Shift+K` | Spotlight command bar |
| `Cmd+Shift+V` | Push-to-Talk (PTT) |

Both configurable via `UserConfig`.

---

## 20. Canonical Defaults Summary

| Area | Decision |
|---|---|
| Ports | OS-assigned random + proxy |
| Vector DB | USearch (`.usearch` index files) |
| Reranker | ONNX MiniLM via `ort` |
| Agent Engine | IronClaw (in-process Rust library) |
| Multimodal | Base64 `image_data` |
| Types | `tauri-specta` |
| Logs | Rolling + 7-day retention |
| Inference | One engine per build (Cargo features) |
| Cloud Storage | 7 providers + local mode via `FileStore` |
| Updates | `tauri-plugin-updater` |
| Database | `openclaw.db` (SQLite via sqlx) |
| Timestamps | Milliseconds (Unix epoch) |
