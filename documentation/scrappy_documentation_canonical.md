# Scrappy – Canonical Technical Documentation (Resolved & Final)

This document is the **single source of truth** for Scrappy’s architecture, implementation, and operational decisions. It resolves all detected inconsistencies across previous documents and fills the remaining technical gaps (reranking, multimodal vision, updates, logging, type safety, vector storage).

Scrappy is a **personal‑only, local‑first desktop application** built with **Tauri (frontend)**, **Rust (backend)**, and **llama.cpp via llama‑server (sidecar)**.

---

## 1. Canonical Architecture Overview

### Process Model

```
Frontend (WebView / JS)
   ↓ invoke()
Rust Backend (Tauri Commands)
   ↓ authenticated proxy (localhost)
llama-server (sidecar, random port)
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

1. Rust selects free port (`pick_free_port()`)
2. Rust generates random token
3. Spawn `llama-server` with:
   - `--host 127.0.0.1`
   - `--port <random>`
   - `--api-key <token>`
4. Poll `/health` until ready
5. Mark model "ready" in UI

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
- conversations
- messages
- jobs
- job_runs
- documents
- chunks
- embeddings
- profiles
- settings

### Full‑Text Search

- SQLite FTS5
- BM25 ranking
- Used for lexical retrieval

---

## 6. Vector Storage (FINAL DECISION)

### Chosen Technology: `sqlite-vec`

**Why:**
- Same DB file as text
- No external services
- Simple backups & export

### Strategy

- Store embeddings in vec0 virtual table
- Chunk ID as foreign key
- Provide “Rebuild Vector Index” tool

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

## 8. Reranking (CRITICAL FIX)

### Implementation (Authoritative)

- **NO Python sidecar**
- **NO llama-server reranking**

### Solution

Run a **small ONNX cross‑encoder** in‑process using **`ort` (ONNX Runtime)**.

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

## 11. Profiles System (FINAL)

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

## 12. Auto‑Tune Suggestions Engine

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
| Vector DB | SQLite + sqlite-vec |
| Reranker | ONNX MiniLM via ort |
| Multimodal | Base64 image_data |
| Types | tauri-specta |
| Logs | Rolling + retention |

---

## Status

With this document, Scrappy is **100% architecturally ready for implementation**.

This file supersedes all prior conflicting instructions.

