# Scrappy – Canonical Technical Documentation (Resolved & Final)

This document is the **single source of truth** for Scrappy’s architecture, implementation, and operational decisions. It resolves all detected inconsistencies across previous documents and fills the remaining technical gaps (reranking, multimodal vision, updates, logging, type safety, vector storage).

Scrappy is a **personal‑only, local‑first desktop application** built with **Tauri (frontend)**, **Rust (backend)**, and **llama.cpp via llama‑server (sidecar)**.

---

## 1. Canonical Architecture Overview

### Process Model

```
Frontend (WebView / JS)
   ↓ invoke()
Rust Backend (Tauri Commands + SidecarManager State)
   ↓ authenticated proxy (localhost)
Sidecar Ecosystem:
   - llama-server (Chat + Context)
   - llama-server (Embedding Indexer)
   - llama-server (Context Summarizer)
   - whisper-server (STT / Voice)
   - Node sidecar (Agent Gateway/Host)
```

**Hard rule:**
> The frontend NEVER talks directly to `llama-server`.

All requests go through Rust. This guarantees:
- security (token never exposed)
- no port conflicts
- consistent request shaping

---

## 2. Sidecar Lifecycle & Security (FINAL)

### Port & Token Policy (Authoritative)

- Bind address: `127.0.0.1`
- Port: **random free port**, chosen at runtime
- Auth: **random per‑launch API token**
- Storage: in‑memory only (never persisted)

### Spawn Flow

1. Rust selects free port via `SidecarManager`.
2. Rust generates random 32-character Alphanumeric token.
3. Spawn sidecar (`llama-server`, `whisper-server`, etc.) with host, port, and token.
4. Track process via `ProcessTracker` (PID management).
5. Poll `/health` or monitor STDOUT for "started" signals.
6. Emit `sidecar_event` for progress tracking (e.g., "prompt processing progress").
7. Mark model "ready" in UI.

### Smart Discovery (Vision)
The manager automatically detects vision projectors (`.mmproj`) by scanning the model's subfolder for files matching the GGUF name or containing "mmproj" flags, ensuring zero-config multimodal support.

### Failure Handling

- Exponential backoff restart
- Crash loop detection
- Safe Mode fallback:
  - `np=1`
  - reduced context
  - CPU fallback (if available)

### UX Requirements

- “Model starting…” progress state
- Unified diagnostics panel:
  - model missing
  - GPU unavailable
  - OOM on load
  - invalid flags

---

## 3. Concurrency & Scheduler (FINAL)

### Two‑Lane Scheduler

| Lane | Purpose | Priority |
|----|----|----|
| Interactive | Chats | Highest |
| Background | RAG ingest, jobs | Lower |

### Rules

- Always reserve **1 slot for chat**
- Default `np=2` (16GB VRAM)
- Background uses slot 2 **only if**:
  - chat idle, OR
  - user enables “Allow background while chatting”

### Cancellation Model

- Stop = close stream + cancel queued task
- Pause job = cancel + persist state
- Resume = requeue from last checkpoint

---

## 4. Streaming UX (FINAL)

### Rendering Strategy

- Buffer tokens
- DOM update cadence: **30–60ms** or sentence boundary
- Virtualized message list

### Message States

- Queued (waiting for slot)
- Running (streaming)
- Throttled (shared compute)
- Cancelled

### UX Requirements

- Always-visible Stop button
- No UI reflow during streaming

---

## 5. Data Layer (FINAL)

### SQLite as Single Source of Truth

Tables:
- **projects**: High-level containers for logical partitioning of work.
- conversations (with project_id foreign key)
- messages (with conversation_id)
- jobs / job_runs
- documents / chunks / embeddings (linked to project_id)
- profiles / settings
- generated_images (history and metadata)

### Partitioning Strategy
- Conversations and RAG documents are scoped to a **Project**.
- Deleting a project triggers a cascading cleanup of all associated transcripts, chunks, and vector indices.

### Full‑Text Search

- SQLite FTS5
- BM25 ranking
- Used for lexical retrieval

---

## 6. Vector Storage

### Technology: `usearch`

**Why:**
- High-performance vector search engine.
- Direct Rust integration via `usearch` crate.
- Managed as an external index file (`.usearch`) alongside SQLite.

### Strategy

- Store embeddings in a `.usearch` file.
- Document chunks stored in SQLite with IDs corresponding to vector indices.
- Provide "Check Vector Index Integrity" tool for synchronization verification.

---

## 7. RAG Pipeline (FINAL)

### Ingestion

1. Extract text (PDF/MD/TXT)
2. Clean headers/footers
3. Chunk (300–800 tokens, 10–20% overlap)
4. Hash chunk
5. Generate embedding
6. Store text + embedding

### Retrieval Pipeline

1. Lexical search (FTS5)
2. Vector search (sqlite-vec)
3. Merge + dedupe
4. Rerank (see below)
5. Token‑budgeted context pack
6. Generate answer with citations

### Context Rules

- Strict token budget
- Neighbor chunk expansion
- Source IDs embedded in prompt

---

## 8. Reranking

### Implementation

- **In-Process:** Run a small ONNX cross-encoder in-process using **`ort` (ONNX Runtime)**.
- **Graciously Handled:** Using `RerankerWrapper` to handle initialization and optional download of models without blocking the main app.

### Model

- `cross-encoder/ms-marco-MiniLM-L-6-v2` (ONNX)
- ~20–30MB

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

## 10. Multimodal Vision Pipeline (FINAL)

### Supported Models

- Pixtral, other mmproj‑based models

### Workflow

1. User drags/pastes image
2. Frontend shows preview
3. Rust backend:
   - stores image in app data
   - resizes/compresses
   - base64 encodes
4. Inject payload:

```json
{
  "content": "Describe this image: [img-1]",
  "image_data": [{ "id": 1, "data": "<base64>" }]
}
```

### Notes

- Frontend never handles base64
- Rust owns conversion & memory

---

## 11. Imagine Studio

### Provider Hybrid Model
- **Local Diffusion**: Power by `sd-sidecar` for fully offline generation. Supports manual resolution of VAE/CLIP components and real-time sampling progress.
- **Nano Banana (Gemini)**: High-speed cloud generation via Google Gemini Imagen 3.
  - `Nano Banana`: Optimized for speed (Gemini 2.5 Flash).
  - `Nano Banana Pro`: Professional-grade with reasoning/grounding (Gemini 3 Pro).

### Key Features
- **Premium Performance**: Default 512px base resolution for fast iterations.
- **Resolution Control**: User-selectable 1K (1024px) and 4K (2048px/interpolated) outputs.
- **Integrated Gallery**: High-performance persistent storage for creations with Favoriting, Search, and **Settings Restoration** (one-click restore of prompt/styles).
- **Bespoke Iconography**: Custom `ImagineModeIcon`, `ImagineSendIcon`, and `ImagineMainIcon` with motion particles and gradients.

Profile =
- model path
- server flags
- ctx
- np
- batching
- RAG config
- sampling preset

### Safety

- Apply → restart → healthcheck
- Failure → automatic rollback

---

## 12. Agent Engine (OpenClaw / OpenClawEngine)

### Process Model
The agent runtime is split into two specialized Node sidecars managed by Rust:
1. **Gateway**: Acts as the communication bridge. Owns Slack/Telegram Socket Mode connections and exposes an authenticated WebSocket control plane to Scrappy.
2. **Node Host**: The execution engine. Runs the agent's main loop, tool execution (Node skills), and human-in-the-loop (HITL) coordination.

### Secrets Firewall (Privacy & Security)
Scrappy implements a strict granular permissions model for external agents:
- **Keys in Escrow**: API keys for Anthropic, OpenAI, etc., are stored in Scrappy's secure identity store.
- **Explicit Grant**: Agents **cannot** see these keys by default. The user must explicitly toggle the "Grant Access" flag per secret in the Settings panel to expose it to the OpenClawEngine environment.

### Workspace Isolation
- Each agent operates in a dedicated workspace directory (`OpenClaw/workspace`).
- Transcripts and artifacts created by agents are isolated from the main chat history to prevent context pollution.

---

## 13. Intelligent Search Grounding

### Logic
Scrappy features a **Search Grounding** layer that monitors user queries for factual indicators (dates, prices, specific entity questions).

### Behavior
- **Auto-Trigger**: Even if not explicitly requested, the system can formalize a query and route it through the `DDGSearchTool` + Chromium Scraper.
- **Exception handling**: Common greetings or abstract philosophical queries bypass grounding to conserve local/API resources.

---

## 14. Auto‑Tune Suggestions Engine

### Triggers

- OOM
- Low tokens/sec
- Queue wait > threshold
- RAG latency spike

### Actions (suggested, never automatic)

- reduce ctx
- reduce np
- switch quant
- disable rerank temporarily
- lower top‑k

### UX Rule

> Never silently change settings.

---

## 13. Model Downloads (Integrated)

- Download GGUF models from Hugging Face
- Resume support
- Integrity check
- Background downloads
- Stored outside app install dir

---

## 14. Type Safety (DX FIX)

### Required Tool

- `tauri-specta`

### Workflow

- Rust structs annotated
- TS bindings auto‑generated
- Frontend build fails on mismatch

---

## 15. Updates Strategy

### Current (Personal‑Only)

- Manual update path
- Update UI placeholder

### Planned

- `tauri-plugin-updater`
- GitHub releases

---

## 16. Logging & Debug Bundles

### Logging

- `tracing`
- `tracing-appender` rolling files
- Retention: last 7 days

### Debug Bundle

Includes:
- version + OS
- profiles
- logs
- RAG stats

Excludes:
- model weights
- raw chat (optional redact)

---

## 17. Canonical Defaults Summary

| Area | Decision |
|----|----|
| Ports | Random + proxy |
| Vector DB | USearch Index (`.usearch`) |
| Reranker | ONNX MiniLM via ort |
| Multimodal | Base64 image_data |
| Types | tauri-specta |
| Logs | Rolling + retention |

---

## Status

With this document, Scrappy is **100% architecturally ready for implementation**.

This file supersedes all prior conflicting instructions.

