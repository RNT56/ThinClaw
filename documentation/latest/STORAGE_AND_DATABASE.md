# Storage & Database Architecture — Technical Analysis

> **Document Version:** 1.0  
> **Date:** 2026-02-24  
> **Scope:** Complete analysis of all data storage, persistence, and retrieval systems  

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Storage Layer Overview](#2-storage-layer-overview)
3. [SQLite Database — Core Relational Store](#3-sqlite-database--core-relational-store)
   - 3.1 [Database Initialization & Pool](#31-database-initialization--pool)
   - 3.2 [Schema — Complete Table Reference](#32-schema--complete-table-reference)
   - 3.3 [Migration History](#33-migration-history)
   - 3.4 [Full-Text Search (FTS5)](#34-full-text-search-fts5)
4. [Conversations & Messages](#4-conversations--messages)
   - 4.1 [Data Model](#41-data-model)
   - 4.2 [Write Path](#42-write-path)
   - 4.3 [Read Path](#43-read-path)
   - 4.4 [Edit & Delete Semantics](#44-edit--delete-semantics)
   - 4.5 [JSON-Serialized Columns](#45-json-serialized-columns)
5. [Projects System](#5-projects-system)
   - 5.1 [Data Model](#51-data-model)
   - 5.2 [Cascade Deletion](#52-cascade-deletion)
6. [RAG Pipeline — Document Ingestion & Retrieval](#6-rag-pipeline--document-ingestion--retrieval)
   - 6.1 [Document Upload & Storage](#61-document-upload--storage)
   - 6.2 [Content Extraction](#62-content-extraction)
   - 6.3 [Chunking Strategy](#63-chunking-strategy)
   - 6.4 [Embedding Generation](#64-embedding-generation)
   - 6.5 [Dual Storage — SQLite + Vector Index](#65-dual-storage--sqlite--vector-index)
   - 6.6 [Retrieval Pipeline (Hybrid Search)](#66-retrieval-pipeline-hybrid-search)
   - 6.7 [Reranking](#67-reranking)
   - 6.8 [Integrity Checks](#68-integrity-checks)
7. [Vector Store — USearch Engine](#7-vector-store--usearch-engine)
   - 7.1 [Architecture](#71-architecture)
   - 7.2 [Scoping Model](#72-scoping-model)
   - 7.3 [Index Parameters](#73-index-parameters)
   - 7.4 [Persistence](#74-persistence)
   - 7.5 [Multi-Scope Search](#75-multi-scope-search)
8. [Image Storage System](#8-image-storage-system)
   - 8.1 [Chat Image Uploads](#81-chat-image-uploads)
   - 8.2 [Image Generation (Diffusion)](#82-image-generation-diffusion)
   - 8.3 [Imagine Gallery — Generated Image Metadata](#83-imagine-gallery--generated-image-metadata)
   - 8.4 [Preview / Thumbnail Storage](#84-preview--thumbnail-storage)
9. [Model Management & Catalog](#9-model-management--catalog)
   - 9.1 [Local Model Files](#91-local-model-files)
   - 9.2 [Remote Model Catalog (DB)](#92-remote-model-catalog-db)
   - 9.3 [Download Manager](#93-download-manager)
   - 9.4 [Standard Assets](#94-standard-assets)
10. [Secret Storage — macOS Keychain Integration](#10-secret-storage--macos-keychain-integration)
    - 10.1 [Architecture](#101-architecture)
    - 10.2 [Storage Model](#102-storage-model)
    - 10.3 [Runtime Flow](#103-runtime-flow)
    - 10.4 [Migration Paths](#104-migration-paths)
    - 10.5 [SecretStore — App-Wide API](#105-secretstore--app-wide-api)
    - 10.6 [Security Properties](#106-security-properties)
11. [Configuration Storage](#11-configuration-storage)
    - 11.1 [User Configuration](#111-user-configuration)
    - 11.2 [OpenClaw Identity](#112-openclaw-identity)
    - 11.3 [OpenClaw Engine Config](#113-openclaw-engine-config)
    - 11.4 [Auth Profiles](#114-auth-profiles)
12. [OpenClaw Engine — Separate State Directory](#12-openclaw-engine--separate-state-directory)
    - 12.1 [Directory Layout](#121-directory-layout)
    - 12.2 [Session & Memory Storage](#122-session--memory-storage)
13. [Filesystem Layout — Complete Directory Map](#13-filesystem-layout--complete-directory-map)
14. [Data Flow Diagrams](#14-data-flow-diagrams)
    - 14.1 [Message Send Flow](#141-message-send-flow)
    - 14.2 [Document Ingestion Flow](#142-document-ingestion-flow)
    - 14.3 [RAG Retrieval Flow](#143-rag-retrieval-flow)
    - 14.4 [Secret Storage Flow](#144-secret-storage-flow)
15. [Concurrency & Thread Safety](#15-concurrency--thread-safety)
16. [Known Limitations & Technical Debt](#16-known-limitations--technical-debt)

---

## 1. Executive Summary

Scrappy employs a **multi-layer storage architecture** combining five distinct persistence mechanisms:

| Layer | Technology | Purpose | Location |
|-------|-----------|---------|----------|
| **Relational DB** | SQLite 3 (via `sqlx`) | Conversations, messages, documents, chunks, projects, image metadata, model catalog | `app_data/openclaw.db` |
| **Vector Index** | USearch (HNSW) | Nearest-neighbor embedding search for RAG | `app_data/vectors/*.usearch` |
| **Filesystem** | Flat files | Uploaded documents, images, model weights, previews, config JSON | `app_data/{documents,images,models,...}` |
| **macOS Keychain** | `security-framework` | API keys & secrets (encrypted at rest) | System Keychain |
| **In-Memory Caches** | `Mutex<HashMap>`, `RwLock`, `Arc` | API key cache, Rig agent cache, config cache, vector store cache | Process memory |

All storage operations are coordinated through **Tauri managed state** — each storage subsystem is registered as a singleton in `lib.rs::run()` and injected into command handlers via `State<'_, T>`.

---

## 2. Storage Layer Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        Frontend (React)                         │
│        ┌─── IPC (Tauri Commands) ──── Event Emitter ──┐        │
└────────┼──────────────────────────────────────────────┼────────┘
         │                                              │
         ▼                                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Backend (Rust / Tauri)                     │
│                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ SqlitePool   │  │ VectorStore  │  │  SecretStore          │  │
│  │ (sqlx)       │  │ Manager      │  │  (Keychain + RwLock)  │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
│         │                 │                      │              │
│  ┌──────▼───────┐  ┌──────▼───────┐  ┌──────────▼───────────┐  │
│  │ openclaw.db  │  │ *.usearch    │  │ macOS Keychain       │  │
│  │ (SQLite 3)   │  │ (USearch)    │  │ (encrypted)          │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
│                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ ConfigMgr    │  │ OpenClawMgr  │  │ Filesystem           │  │
│  │ (JSON)       │  │ (JSON + WS)  │  │ (docs/imgs/models)   │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. SQLite Database — Core Relational Store

### 3.1 Database Initialization & Pool

**File:** `backend/src/lib.rs` (lines 317–380)

```rust
// Database file location
let db_path = app_data_dir.join("openclaw.db");

// Legacy migration: rename scrappy.db → openclaw.db
if !db_path.exists() && legacy_db.exists() {
    let _ = fs::rename(&legacy_db, &db_path);
}

// Connection pool: 5 concurrent connections, WAL mode (implicit)
let pool = SqlitePoolOptions::new()
    .max_connections(5)
    .connect(&db_url)
    .await
    .expect("failed to connect to database");

// Run all pending migrations
sqlx::migrate!("./migrations")
    .run(&pool)
    .await
    .expect("failed to run migrations");

// Register as Tauri managed state
handle.manage(pool);
```

**Key Properties:**
- **ORM:** `sqlx` (compile-time checked queries via `query_as!`)
- **Connection mode:** `mode=rwc` (read-write-create)
- **Pool size:** 5 connections (sufficient for a desktop app)
- **WAL mode:** Enabled implicitly by SQLite for concurrent reads/writes
- **Managed as:** `State<'_, SqlitePool>` in all Tauri commands

### 3.2 Schema — Complete Table Reference

#### `settings`
```sql
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```
Simple key-value store. Currently used minimally — most settings are stored in `user_config.json`.

#### `conversations`
```sql
CREATE TABLE conversations (
    id          TEXT PRIMARY KEY,        -- 16-char alphanumeric
    title       TEXT NOT NULL,
    created_at  INTEGER NOT NULL,        -- Unix epoch seconds
    updated_at  INTEGER NOT NULL,        -- Updated on each new message
    project_id  TEXT REFERENCES projects(id) ON DELETE SET NULL,
    sort_order  INTEGER DEFAULT 0        -- Manual drag-and-drop ordering
);
CREATE INDEX idx_conversations_project_id ON conversations(project_id);
```

#### `messages`
```sql
CREATE TABLE messages (
    id                 TEXT PRIMARY KEY,     -- 16-char alphanumeric
    conversation_id    TEXT NOT NULL,
    role               TEXT NOT NULL,        -- 'user', 'assistant', 'system'
    content            TEXT NOT NULL,
    images             TEXT,                 -- JSON: ["uuid1","uuid2"]
    attached_docs      TEXT,                 -- JSON: [{"id":"...","name":"..."}]
    web_search_results TEXT,                 -- JSON: [{title,link,snippet}]
    created_at         INTEGER NOT NULL,     -- Unix epoch seconds
    FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);
```

#### `documents`
```sql
CREATE TABLE documents (
    id          TEXT PRIMARY KEY,        -- 16-char alphanumeric
    path        TEXT NOT NULL,           -- Absolute filesystem path
    hash        TEXT NOT NULL,           -- SHA-256 of file content
    status      TEXT NOT NULL,           -- 'indexed', 'processing', 'error'
    created_at  INTEGER NOT NULL,        -- Unix epoch milliseconds
    updated_at  INTEGER NOT NULL,
    chat_id     TEXT,                    -- Scoped to a standalone chat
    project_id  TEXT REFERENCES projects(id) ON DELETE SET NULL
);
CREATE INDEX idx_documents_chat_id    ON documents(chat_id);
CREATE INDEX idx_documents_project_id ON documents(project_id);
```

#### `chunks`
```sql
CREATE TABLE chunks (
    id           TEXT PRIMARY KEY,       -- "{doc_id}-{chunk_index}"
    document_id  TEXT NOT NULL,
    content      TEXT NOT NULL,          -- Actual text content
    chunk_index  INTEGER NOT NULL,       -- Position within document
    embedding    BLOB,                   -- Raw f32 LE bytes (backup)
    FOREIGN KEY(document_id) REFERENCES documents(id) ON DELETE CASCADE
);
```

#### `chunks_fts` (FTS5 Virtual Table)
```sql
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    content,
    chunk_uuid UNINDEXED,       -- Stored but not full-text indexed
    content='chunks',            -- External content table
    content_rowid='rowid'        -- Maps FTS rowid to chunks.rowid
);
```
Synchronized via three triggers (`chunks_ai`, `chunks_ad`, `chunks_au`) that keep the FTS index in sync with the `chunks` table on INSERT, DELETE, and UPDATE.

#### `projects`
```sql
CREATE TABLE projects (
    id          TEXT PRIMARY KEY,        -- UUIDv4
    name        TEXT NOT NULL,
    description TEXT,
    created_at  INTEGER NOT NULL,        -- Unix epoch milliseconds
    updated_at  INTEGER NOT NULL,
    sort_order  INTEGER DEFAULT 0
);
```

#### `chat_summaries`
```sql
CREATE TABLE chat_summaries (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL UNIQUE,
    summary         TEXT NOT NULL,
    embedding       BLOB,               -- Future: summary embeddings for semantic search
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);
```

#### `models_catalog`
```sql
CREATE TABLE models_catalog (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    metadata        TEXT NOT NULL,       -- Full JSON blob (size, quant, family, etc.)
    local_version   TEXT,
    remote_version  TEXT,
    last_checked_at INTEGER,
    status          TEXT                 -- 'installed', 'outdated', 'unavailable'
);
```

#### `generated_images`
```sql
CREATE TABLE generated_images (
    id             TEXT PRIMARY KEY NOT NULL,
    prompt         TEXT NOT NULL,
    style_id       TEXT,
    provider       TEXT NOT NULL DEFAULT 'local',     -- 'local', 'nano-banana', 'nano-banana-pro'
    aspect_ratio   TEXT NOT NULL DEFAULT '1:1',
    resolution     TEXT,                              -- '512', '1K', '2K', '4K'
    width          INTEGER,
    height         INTEGER,
    seed           INTEGER,
    file_path      TEXT NOT NULL,                     -- Absolute path to PNG
    thumbnail_path TEXT,
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    is_favorite    INTEGER NOT NULL DEFAULT 0,
    tags           TEXT                               -- Future: comma-separated tags
);
CREATE INDEX idx_generated_images_created_at  ON generated_images(created_at DESC);
CREATE INDEX idx_generated_images_provider    ON generated_images(provider);
CREATE INDEX idx_generated_images_is_favorite ON generated_images(is_favorite);
```

### 3.3 Migration History

| Migration | Date | Description |
|-----------|------|-------------|
| `20240101000000_init.sql` | 2024-01-01 | Base schema: `settings`, `conversations`, `messages`, `documents`, `chunks` |
| `20240101000001_fts.sql` | 2024-01-01 | FTS5 virtual table + sync triggers for `chunks` |
| `20240115000001_add_images_to_messages.sql` | 2024-01-15 | Add `images TEXT` column to `messages` |
| `20260116000000_add_chat_id_to_documents.sql` | 2026-01-16 | Scoped RAG: `chat_id` column + index on `documents` |
| `20260116205500_add_attached_docs.sql` | 2026-01-16 | Add `attached_docs TEXT` column to `messages` |
| `20260117000000_add_projects.sql` | 2026-01-17 | `projects` table, `project_id` FK on conversations+documents, `chat_summaries` table |
| `20260119000000_add_web_search_results.sql` | 2026-01-19 | Add `web_search_results TEXT` column to `messages` |
| `20260127000000_add_reordering.sql` | 2026-01-27 | Add `sort_order` columns to `conversations` and `projects` |
| `20260208000000_model_catalog.sql` | 2026-02-08 | `models_catalog` table for HF Hub model tracking |
| `20260209000000_generated_images.sql` | 2026-02-09 | `generated_images` table with indexes for Imagine gallery |

### 3.4 Full-Text Search (FTS5)

The FTS5 index enables keyword search across document chunks with SQLite's built-in tokenizer:

```sql
-- Search query (quoted phrase for exact matching)
SELECT rowid FROM chunks_fts
WHERE content MATCH '"user query"'
ORDER BY rank
LIMIT 150;
```

**Synchronization:** Three `AFTER INSERT/DELETE/UPDATE` triggers on the `chunks` table automatically maintain the FTS index. The `content=` and `content_rowid=` options use the external content table pattern, meaning FTS shares content with the base table (no duplication) and maps via SQLite's internal `rowid`.

---

## 4. Conversations & Messages

**File:** `backend/src/history.rs`

### 4.1 Data Model

```rust
pub struct Conversation {
    pub id: String,          // 16-char alphanumeric (rand)
    pub title: String,
    pub created_at: i64,     // Unix epoch seconds
    pub updated_at: i64,     // Bumped on every new message
    pub project_id: Option<String>,
    pub sort_order: i32,     // Manual reordering support
}

pub struct MessageEntry {
    pub id: String,                        // 16-char alphanumeric
    pub conversation_id: String,
    pub role: String,                      // "user" | "assistant" | "system"
    pub content: String,                   // Markdown text
    pub images: Option<String>,            // JSON array of image UUIDs
    pub attached_docs: Option<String>,     // JSON array of AttachedDoc
    pub web_search_results: Option<String>, // JSON array of WebSearchResult
    pub created_at: i64,
}
```

### 4.2 Write Path

```
Frontend chat → commands::save_message()
    ├── Generate 16-char alphanumeric ID
    ├── Serialize images → JSON string (Option<Vec<String>>)
    ├── Serialize attached_docs → JSON string (Option<Vec<AttachedDoc>>)
    ├── Serialize web_search_results → JSON string
    ├── INSERT INTO messages (...)
    └── UPDATE conversations SET updated_at = now WHERE id = ?
```

**Transaction Safety:** Individual message saves are NOT wrapped in explicit transactions — they rely on SQLite's implicit transaction per statement. However, `edit_message` and `update_conversations_order` DO use explicit transactions.

### 4.3 Read Path

```
Frontend → commands::get_messages(conversation_id, limit?, before_created_at?)
    ├── SELECT * FROM messages WHERE conversation_id=? AND created_at < ?
    │   ORDER BY created_at DESC LIMIT ?
    ├── Reverse results (DESC→ASC for chronological display)
    ├── Deserialize images JSON → Option<Vec<String>>
    ├── Deserialize attached_docs JSON (with legacy format fallback)
    ├── Deserialize web_search_results JSON
    └── Return Vec<FrontendMessage>
```

**Pagination:** Cursor-based pagination using `created_at < before_created_at` with `LIMIT`. Default limit is 50 messages.

**Legacy Format Handling:** The `attached_docs` column supports both the old format (`["id:name"]` string pairs) and the new format (`[{"id":"...","name":"..."}]` objects), with automatic detection and conversion on read.

### 4.4 Edit & Delete Semantics

**Message Editing** (`edit_message`):
1. Fetch the message's `created_at` and `conversation_id`
2. Update the message content
3. **Delete all subsequent messages** in the conversation (chat branching)
4. Update the conversation's `updated_at` timestamp
5. All in a single SQLite transaction

**Conversation Deletion:** `ON DELETE CASCADE` on `messages.conversation_id` ensures all messages are automatically deleted when a conversation is removed.

**Full History Deletion** (`delete_all_history`):
1. Delete all chunks, documents, messages, conversations, chat_summaries, and projects (in that order, in a transaction)
2. Reset all vector store indices (`VectorStoreManager::reset_all()`)
3. Clear filesystem directories (`documents/`, `images/`)

### 4.5 JSON-Serialized Columns

Several `messages` columns store structured data as JSON strings:

| Column | Rust Type | JSON Shape |
|--------|-----------|-----------|
| `images` | `Option<Vec<String>>` | `["uuid1", "uuid2"]` |
| `attached_docs` | `Option<Vec<AttachedDoc>>` | `[{"id":"docId","name":"file.pdf"}]` |
| `web_search_results` | `Option<Vec<WebSearchResult>>` | `[{"title":"...","link":"...","snippet":"..."}]` |

This design avoids additional join tables. Serialization uses `serde_json::to_string()` on write and `serde_json::from_str()` on read with graceful fallback to `None`.

---

## 5. Projects System

**File:** `backend/src/projects.rs`

### 5.1 Data Model

Projects provide organizational grouping for conversations and documents with scoped RAG:

```rust
pub struct Project {
    pub id: String,              // UUIDv4
    pub name: String,
    pub description: Option<String>,
    pub created_at: i64,         // Unix epoch milliseconds
    pub updated_at: i64,
    pub sort_order: i32,
}
```

**Relationships:**
- `conversations.project_id` → `projects.id` (ON DELETE SET NULL)
- `documents.project_id` → `projects.id` (ON DELETE SET NULL)
- Vector indices are scoped per-project (`project_{id}_{dims}.usearch`)

### 5.2 Cascade Deletion

When a project is deleted (`delete_project`), an explicit multi-step transaction runs:

```
1. DELETE messages WHERE conversation_id IN (project conversations)
2. DELETE conversations WHERE project_id = ?
3. DELETE chunks WHERE document_id IN (project documents)  
4. DELETE documents WHERE project_id = ?
5. DELETE projects WHERE id = ?
6. COMMIT transaction
7. Delete the project's scoped vector index file
```

This is intentionally explicit (not relying solely on CASCADE) for clarity and to ensure the vector index file is cleaned up.

---

## 6. RAG Pipeline — Document Ingestion & Retrieval

**File:** `backend/src/rag.rs` (925 lines)

### 6.1 Document Upload & Storage

```
Frontend → upload_document(file_bytes, filename)
    ├── Sanitize filename (extract basename)
    ├── Generate 8-char random prefix
    ├── Save to: app_data/documents/{prefix}_{filename}
    └── Return absolute path
```

Documents are stored as raw files in `app_data/documents/`. The database tracks metadata in the `documents` table.

### 6.2 Content Extraction

**Text Files:** Direct UTF-8 decode with `String::from_utf8_lossy()`.

**PDFs — Multi-Stage Pipeline:**

```
1. Primary: pdf_extract::extract_text()
2. Quality Check: Garbage detection
   ├── Empty content → force OCR
   ├── < 25% alphanumeric characters → garbage
   ├── File > 50KB but < 1000 chars → scan-like
3. Fallback: Vision-OCR via headless Chromium
   ├── Launch Chromium (chromiumoxide)
   ├── Open PDF as file:// URL
   ├── For each page (up to 15):
   │   ├── Screenshot (JPEG, quality 85)
   │   ├── Base64 encode
   │   ├── Send to local vision LLM (chat server)
   │   │   with prompt: "Transcribe all visible text..."
   │   ├── Append transcription to OCR content
   │   └── Stop if page returns "[empty]"
   └── Combine: original text + OCR supplemental content
```

**Preview Generation:** First page of every PDF is rendered as a JPEG screenshot and saved to `app_data/previews/{hash}.jpg`.

### 6.3 Chunking Strategy

- **Chunk size:** 1000 characters (configurable in principle, hardcoded in practice)
- **Overlap:** 100 characters between adjacent chunks
- **Algorithm:** Sliding window over character array (Unicode-aware via `chars()`)
- **Empty handling:** Single empty chunk for documents with no extractable content

### 6.4 Embedding Generation

Embeddings are generated using the local embedding sidecar server:

```
POST http://127.0.0.1:{embedding_port}/v1/embeddings
Authorization: Bearer {token}
{
    "input": "<chunk text>",
    "model": "default"
}
→ Response: { "data": [{ "embedding": [f32; 384], "index": 0 }] }
```

- **Default dimensions:** 384 (configurable via `UserConfig.vector_dimensions`)
- **Concurrency:** 5 parallel embedding requests (`buffer_unordered(5)`)
- **Model:** e5-small or similar via llama-embedding-server sidecar

### 6.5 Dual Storage — SQLite + Vector Index

Each chunk is stored in **two** locations:

1. **SQLite `chunks` table:** Full text content + raw embedding bytes (BLOB backup)
2. **USearch vector index:** Embedding vector indexed by SQLite `rowid` for ANN search

```rust
// 1. Insert chunk + get its SQLite rowid
let rowid: i64 = sqlx::query_scalar(
    "INSERT INTO chunks (...) VALUES (?, ?, ?, ?, ?) RETURNING rowid"
).fetch_one(&pool);

// 2. Add embedding vector to USearch, keyed by the rowid
scoped_store.add(rowid as u64, &embedding_vector)?;
```

**Consistency:** If embedding or vector index insertion fails, the document record is rolled back (`DELETE FROM documents WHERE id = ?`). On success, the vector index is saved to disk.

### 6.6 Retrieval Pipeline (Hybrid Search)

**File:** `rag.rs::retrieve_context_internal()`

The retrieval system uses **Reciprocal Rank Fusion (RRF)** to combine:

```
                  ┌─── Vector Search (ANN) ───────────────┐
                  │  Embed query → USearch search          │
                  │  Returns: top-150 rowids by cosine     │
                  │  Scopes: [project/chat] + Global       │
User Query ──────│                                        ├──→ RRF Merge ──→ Rerank ──→ Top 5
                  │                                        │
                  └─── FTS Search (BM25) ────────────────┐│
                       chunks_fts MATCH "query"           ││
                       Returns: top-150 rowids by rank     │
                       ──────────────────────────────────────┘
```

**RRF Formula:** `score(d) = Σ 1 / (k + rank + 1)` where `k = 60`

**Scope Resolution:**
1. Determine project_id from chat's conversation record (if in project scope)
2. Build search scopes: `[Project(pid)]` or `[Chat(cid)]` + always `[Global]`
3. Vector search across all applicable scopes
4. FTS search has no scope filtering (searches all chunks)

**Special Cases:**
- **Overview queries** ("list files", "what documents"): Return file paths directly
- **Filename queries** ("about file.pdf"): Return full document content (up to 15K chars)
- **Vague queries** ("summarize", "what is this"): Prepend introductory chunks from attached docs

### 6.7 Reranking

**File:** `backend/src/reranker.rs`

A cross-encoder reranker refines results after RRF fusion:

```
Model: Xenova/ms-marco-MiniLM-L-6-v2 (quantized ONNX)
Runtime: ONNX Runtime (via ort crate)
Tokenizer: HuggingFace tokenizers
Auto-download: On first use to app_data/reranker_model.onnx + reranker_tokenizer.json
```

**Process:**
1. For each candidate: encode `[CLS] query [SEP] document [SEP]`
2. Run through ONNX model → logit score
3. Sort descending by score
4. Filter by threshold (−5.0 for general, −10.0 for explicit doc queries)
5. Return top 5 results

**Graceful Degradation:** `RerankerWrapper` handles initialization failures — if the model fails to download or load, reranking is skipped and candidates are returned in their original RRF order.

### 6.8 Integrity Checks

```rust
// On startup: compare DB chunk count vs loaded vector index count
let chunk_count: i64 = "SELECT COUNT(*) FROM chunks";
let vector_count = vector_manager.total_count()?;
// Report mismatch (indices are lazily loaded, so mismatch on fresh start is expected)
```

---

## 7. Vector Store — USearch Engine

**File:** `backend/src/vector_store.rs`

### 7.1 Architecture

```
VectorStoreManager (Tauri managed state)
    ├── base_dir: app_data/vectors/
    ├── dimensions: 384
    └── stores: Arc<Mutex<HashMap<VectorScope, Arc<VectorStore>>>>
                          ├── Global    → global_384.usearch
                          ├── Project(A) → project_A_384.usearch
                          ├── Chat(B)    → chat_B_384.usearch
                          └── ...
```

### 7.2 Scoping Model

```rust
pub enum VectorScope {
    Global,           // Documents not tied to project/chat
    Project(String),  // Documents in a specific project
    Chat(String),     // Documents in a standalone chat
}

// Resolution logic:
fn scope_for(project_id: &Option<String>, chat_id: &Option<String>) -> VectorScope {
    if project_id is set → VectorScope::Project(pid)
    elif chat_id is set  → VectorScope::Chat(cid)
    else                 → VectorScope::Global
}
```

**File naming:** `{scope}_{dimensions}.usearch`
- `global_384.usearch`
- `project_AbCdEfGh_384.usearch`
- `chat_XyZ12345_384.usearch`

### 7.3 Index Parameters

```rust
IndexOptions {
    dimensions: 384,         // Matches embedding model output
    metric: MetricKind::Cos, // Cosine similarity
    quantization: ScalarKind::F32, // No quantization
    connectivity: 16,        // HNSW M parameter
    expansion_add: 128,      // HNSW efConstruction
    expansion_search: 64,    // HNSW efSearch
    multi: false,            // Single vector per key
}
```

### 7.4 Persistence

- **Lazy initialization:** Indices are created on first access for each scope
- **Load on access:** If the `.usearch` file exists and is non-empty, it's loaded; corrupt files are backed up and reset
- **Save on change:** After each document ingestion, the affected scope's index is saved to disk
- **Auto-capacity:** When the index reaches capacity, it's automatically expanded (`max(cap+1000, cap*2)`)
- **Cleanup:** `reset_all()` deletes all `.usearch` files and clears in-memory state

### 7.5 Multi-Scope Search

```rust
fn search_scoped(&self, vector: &[f32], scopes: &[VectorScope], limit: usize) -> Vec<u64> {
    // Search each scope independently
    // Merge all results
    // Deduplicate by rowid
    // Return sorted list of rowids
}
```

Retrieval always searches the relevant scope + `Global` to ensure globally-uploaded documents are available everywhere.

---

## 8. Image Storage System

### 8.1 Chat Image Uploads

**File:** `backend/src/images.rs`

```
Frontend → upload_image(image_bytes)
    ├── Decode image (image crate)
    ├── Resize if > 1024×1024 (Lanczos3)
    ├── Generate UUIDv4 ID
    ├── Save as: app_data/images/{uuid}.jpg
    └── Return ImageResponse { id, path }
```

**Storage Pattern:** Images are referenced by UUID in message records. The `messages.images` column stores a JSON array of UUIDs. On display, the frontend calls `load_image(id)` which returns base64-encoded data, or `get_image_path(id)` for filesystem access.

**Format Resolution:** When loading, the system checks for `.png` first (SD output), then `.jpg` (upload output).

### 8.2 Image Generation (Diffusion)

**File:** `backend/src/image_gen.rs`

```
Frontend → generate_image(params)
    ├── Resolve model path (params → SidecarManager → error)
    ├── Stop chat server (free GPU VRAM)
    ├── Attempt 1: Strict mode (no standard fallbacks)
    ├── Attempt 2: Fallback mode (try standard components)
    ├── sd sidecar process generates to temp file
    ├── Copy temp PNG → app_data/images/{uuid}.png
    ├── Emit "image_gen_success" event
    └── Return ImageResponse { id, path }
```

**Architecture Detection:** Automatic detection of diffusion model architecture (Flux1, Flux2 Klein, SD 1.5, SD 2.1, SDXL, SD 3.5 Medium/Turbo, Qwen Image, Wan2.1) from the model filename, with architecture-specific parameter tuning.

### 8.3 Imagine Gallery — Generated Image Metadata

**File:** `backend/src/imagine.rs`

The Imagine gallery stores structured metadata for every generated image:

```
imagine_generate(params)
    ├── Route to provider: local | nano-banana | nano-banana-pro
    │   ├── Local: → image_gen::generate_image()
    │   ├── Nano Banana: → Gemini 2.5 Flash Image API
    │   └── Nano Banana Pro: → Gemini 3 Pro Image Preview API
    ├── INSERT INTO generated_images (id, prompt, style_id, provider, ...)
    └── Return GeneratedImage with full metadata
```

**Gallery Queries:**
- `imagine_list_images(limit, offset, favorites_only)` — Paginated gallery with optional favorites filter
- `imagine_search_images(query)` — LIKE search on `prompt` and `tags` columns
- `imagine_toggle_favorite(id)` — Toggle `is_favorite` boolean
- `imagine_delete_image(id)` — Delete file + DB record
- `imagine_get_stats()` — Aggregate counts by provider and favorites

### 8.4 Preview / Thumbnail Storage

PDF document previews are stored separately from other images:

```
app_data/
├── images/            ← Chat uploads + generated images (UUID.jpg/png)
└── previews/          ← PDF first-page screenshots ({sha256_hash}.jpg)
```

---

## 9. Model Management & Catalog

**File:** `backend/src/model_manager.rs`

### 9.1 Local Model Files

Models are stored in `app_data/models/` with a recursive directory structure:

```
app_data/models/
├── model-name.gguf                       ← Single-file GGUF models
├── HuggingFace-Org--Model-Name/          ← Multi-file model bundles
│   ├── config.json
│   ├── model-00001-of-00002.safetensors
│   └── model-00002-of-00002.safetensors
└── standard/                             ← Standard component assets
    ├── vae/
    ├── clip/
    └── t5/
```

**Bundle Detection:** A directory is treated as a model bundle if it contains `config.json` AND at least one weight file (`.safetensors`, `.bin`, `.pt`, `.ckpt`, `.sft`).

**Engine Filtering:** `engine_filter_models()` uses compile-time feature flags to filter models by what the active engine supports:
- `llama_cpp` engine: `.gguf` files only
- `mlx` engine: directories with `config.json` only
- `vllm` / `ollama` engines: support both

### 9.2 Remote Model Catalog (DB)

```rust
pub struct RemoteModelEntry {
    pub id: String,
    pub name: String,
    pub metadata: String,        // Full JSON: size, quant, family, etc.
    pub local_version: Option<String>,
    pub remote_version: Option<String>,
    pub last_checked_at: Option<i64>,
    pub status: Option<String>,  // 'installed', 'outdated', 'unavailable'
}
```

`update_remote_model_catalog()` uses **UPSERT** (`INSERT OR REPLACE`) to sync HF Hub discovery results into the `models_catalog` table.

### 9.3 Download Manager

**File:** `backend/src/model_manager.rs::DownloadManager`

```rust
pub struct DownloadManager {
    cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}
```

- In-memory only — no persistence of download state across restarts
- Supports cancellation via per-filename `AtomicBool` flags
- Progress emitted via Tauri events (`download_progress`)
- Downloads use streaming HTTP with `reqwest::Client`
- Automatic HuggingFace token injection for gated model downloads

### 9.4 Standard Assets

Standard assets (VAE, CLIP, T5 encoders for image generation) are stored in a predefined structure:

```
app_data/models/standard/
├── vae/    ← ae.safetensors (VAE decoder)
├── clip/   ← clip_l.safetensors, clip_g.safetensors
└── t5/     ← t5xxl*.safetensors (text encoder)
```

`check_missing_standard_assets()` compares a hardcoded list of expected files against what's on disk, returning any missing assets.

---

## 10. Secret Storage — macOS Keychain Integration

### 10.1 Architecture

**File:** `backend/src/openclaw/config/keychain.rs` + `backend/src/secret_store.rs`

```
                     ┌──────────────────────────────────────┐
                     │         macOS Keychain               │
                     │  ┌────────────────────────────────┐  │
                     │  │ Service: com.schack.scrappy    │  │
                     │  │ Account: api_keys              │  │
                     │  │ Password: {"anthropic":"sk-..."│  │
                     │  │           ,"openai":"sk-..."   │  │
                     │  │           ,"gemini":"AIza..."   │  │
                     │  │           ...}                  │  │
                     │  └────────────────────────────────┘  │
                     └──────────┬──────────┬────────────────┘
                                │          │
                    load_all()  │          │ set_key() / flush
                    (1× startup)│          │ (on mutation)
                                ▼          ▲
                     ┌──────────────────────────────────────┐
                     │    In-Memory Cache (Mutex<HashMap>)  │
                     │    ├── keychain::key_cache()         │
                     │    └── SecretStore::keys (RwLock)     │
                     └──────────────────────────────────────┘
                                │                    │
                     get_key()  │                    │ SecretStore.get()
                     (no Keychain│ access)            │ (concurrent reads)
                                ▼                    ▼
                     ┌────────────────────┐  ┌─────────────────────┐
                     │  OpenClawConfig    │  │  Rig Agent          │
                     │  (engine config)   │  │  HF Hub Downloader  │
                     │                    │  │  Settings UI         │
                     └────────────────────┘  └─────────────────────┘
```

### 10.2 Storage Model

**Single JSON blob** stored as one Keychain item:
- **Service:** `com.schack.scrappy`
- **Account:** `api_keys`
- **Password (value):** JSON object mapping provider slugs to API keys

This design triggers exactly **one** macOS Keychain authorization prompt on app startup, not one per key.

**Supported Providers:** 21 provider slugs including `anthropic`, `openai`, `openrouter`, `gemini`, `groq`, `brave`, `huggingface`, `xai`, `venice`, `together`, `moonshot`, `minimax`, `nvidia`, `qianfan`, `mistral`, `xiaomi`, `bedrock_access_key_id`, `bedrock_secret_access_key`, `bedrock_region`, `custom_llm_key`, `remote_token`.

### 10.3 Runtime Flow

1. **Startup:** `keychain::load_all()` → single `get_generic_password()` call → populate `key_cache()` HashMap
2. **Read:** `keychain::get_key(provider)` → reads from in-memory cache (zero Keychain calls)
3. **Write:** `keychain::set_key(provider, value)` → update cache → `flush_cache()` → single `set_generic_password()` call
4. **SecretStore:** Higher-level wrapper with `RwLock` for concurrent reads, convenience accessors

### 10.4 Migration Paths

Three migration paths handle historical storage formats:

1. **Per-key Keychain → Unified blob:** Old builds stored each key as a separate Keychain item. On first launch, `migrate_per_key_items()` reads all legacy items, consolidates into the single JSON blob, and deletes the old items. **Only runs if the unified blob doesn't exist.**

2. **Plaintext `identity.json` → Keychain:** Pre-keychain builds stored API keys directly in `identity.json`. `keychain::migrate_from_identity()` reads each key field, stores it in Keychain, and nulls the JSON field. The sanitized `identity.json` is then saved back.

3. **Custom secret values:** Old `identity.json` included `"value"` fields in custom_secrets entries. Since `CustomSecret.value` now uses `#[serde(skip)]`, raw JSON parsing imports these values into Keychain under each secret's ID.

### 10.5 SecretStore — App-Wide API

```rust
pub struct SecretStore {
    keys: RwLock<HashMap<String, String>>,
}
```

**Consumers:**
- **OpenClaw engine** — reads granted keys for `auth-profiles.json` generation
- **HF Hub** — reads HuggingFace token for API calls
- **Rig agent** — reads provider keys for direct inference
- **Model downloader** — reads HF token for gated model downloads
- **Settings UI** — reads/writes keys from the Secrets page

**Security Note:** `snapshot()` method was intentionally removed to prevent unfiltered key access. The OpenClaw engine only receives keys that have been explicitly granted via per-provider `_granted` flags.

### 10.6 Security Properties

- **Encryption at rest:** macOS Keychain uses the user's login password / Secure Enclave
- **Memory zeroization:** `OpenClawConfig` implements `Drop` with `zeroize::Zeroize` to overwrite all API key fields with `0x00` before deallocation
- **No plaintext on disk:** API keys are never written to JSON files — only boolean `_granted` flags appear in `identity.json`
- **Custom secret isolation:** Custom secrets map their `id` to a Keychain entry, with `#[serde(skip)]` preventing JSON serialization

---

## 11. Configuration Storage

### 11.1 User Configuration

**File:** `backend/src/config.rs` → `UserConfig`  
**Location:** `app_config/user_config.json`

```rust
pub struct UserConfig {
    // Web search & scraping
    pub search_concurrency_limit: u32,    // default: 2
    pub scrape_concurrency_limit: u32,    // default: 2
    pub max_search_results: u32,          // default: 5
    pub max_scrape_chars: u32,            // default: 15000
    pub scrape_timeout_secs: u32,         // default: 30
    // LLM & context
    pub default_context_window: u32,      // default: 8192
    pub summarization_chunk_size: u32,    // default: 4000
    pub llm_temperature: f32,             // default: 0.7
    pub llm_top_p: f32,                   // default: 0.9
    // Vector store
    pub vector_dimensions: u32,           // default: 384
    // Image generation
    pub sd_threads: u32,                  // default: 0 (auto)
    // Personalization
    pub knowledge_bits: Vec<KnowledgeBit>,
    pub custom_personas: Vec<CustomPersona>,
    pub selected_persona: String,
    pub selected_chat_provider: Option<String>,
    // Memory management
    pub memory_reservation_gb: u32,       // default: 4
    pub enable_memory_reservation: bool,
    pub mlock: bool,
    pub quantize_kv: bool,
    // MCP integration
    pub mcp_base_url: Option<String>,
    pub mcp_auth_token: Option<String>,
    pub mcp_sandbox_enabled: bool,
    pub mcp_cache_ttl_secs: u32,          // default: 300
    pub mcp_tool_result_max_chars: u32,   // default: 5000
    // ...
}
```

**Persistence Details:**
- In-memory via `ConfigManager(Mutex<UserConfig>)` — reads are instant
- Writes flush to disk asynchronously via `tokio::fs::write`
- **JSON merge semantics:** `update_user_config()` performs a field-level merge (not full replacement) to prevent concurrent partial updates from overwriting each other
- All fields use `#[serde(default)]` for forward compatibility with older config files

### 11.2 OpenClaw Identity

**Location:** `app_data/OpenClaw/state/identity.json`

Stores non-sensitive configuration for the OpenClaw engine subsystem:

```json
{
    "device_id": "scrappy-uuid-here",
    "auth_token": "32-char-random-token",
    "anthropic_granted": true,
    "openai_granted": false,
    "profiles": [...],
    "gateway_mode": "local",
    "setup_completed": true,
    "custom_secrets": [
        {"id": "my-key", "name": "My API Key", "granted": true}
    ]
    // NOTE: NO api key values — those are in Keychain
}
```

### 11.3 OpenClaw Engine Config

**Location:** `app_data/OpenClaw/state/openclaw.json`

Generated configuration file consumed by the OpenClaw Engine process:

```json
{
    "gateway": { "mode": "local", "bind": "0.0.0.0", "port": 18789, "auth": {...} },
    "discovery": { "mdns": { "mode": "disabled" } },
    "agents": { "defaults": { "workspace": "...", "model": {...} } },
    "models": { "providers": { "anthropic": {...}, "local": {...} } },
    "channels": { "slack": {...}, "telegram": {...} },
    "tools": { "allow": [...], "deny": [...] },
    "meta": { "lastTouchedVersion": "2026.2.23-beta.1" }
}
```

### 11.4 Auth Profiles

**Location:** `app_data/OpenClaw/state/auth-profiles.json`

Runtime artifact generated by `write_config()` that maps API keys to providers for the engine:

```json
{
    "anthropic": { "apiKey": "sk-..." },
    "openai": { "apiKey": "sk-..." }
}
```

**Security:** Only keys with their corresponding `*_granted` flag set to `true` are written to this file. This prevents the engine from accessing keys the user hasn't explicitly authorized.

---

## 12. OpenClaw Engine — Separate State Directory

### 12.1 Directory Layout

```
app_data/OpenClaw/
├── state/
│   ├── identity.json         ← Non-sensitive identity & config flags
│   ├── openclaw.json         ← Engine runtime config (regenerated)
│   ├── auth-profiles.json    ← Granted API keys (runtime artifact)
│   └── sessions/             ← OpenClaw chat session state (managed by engine)
├── workspace/                ← Agent workspace (file operations, code execution)
└── logs/                     ← Engine process logs
```

### 12.2 Session & Memory Storage

OpenClaw sessions are managed by the engine process (Node.js), not by the Rust backend:

- **Sessions:** Stored in `state/sessions/` as JSON files per session
- **Memory:** Agent memory managed by the engine's internal state
- **Accessed via RPC:** The Rust backend reads/writes session data through WebSocket RPC calls to the engine

---

## 13. Filesystem Layout — Complete Directory Map

```
$APP_DATA_DIR/                              ← Tauri's app_data_dir()
├── openclaw.db                             ← SQLite database (primary)
├── user_config.json                        ← User preferences (app_config_dir)
├── documents/                              ← Uploaded document files
│   └── {random8}_{original_filename}
├── images/                                 ← All image files
│   ├── {uuid}.jpg                          ← Chat image uploads (resized)
│   └── {uuid}.png                          ← Generated images (SD/Gemini)
├── previews/                               ← PDF first-page screenshots
│   └── {sha256_hash}.jpg
├── vectors/                                ← USearch vector index files
│   ├── global_384.usearch
│   ├── project_{id}_384.usearch
│   └── chat_{id}_384.usearch
├── models/                                 ← ML model weights
│   ├── *.gguf                              ← Single-file GGUF models
│   ├── Org--Model-Name/                    ← Multi-file model bundles
│   └── standard/                           ← Standard diffusion components
│       ├── vae/
│       ├── clip/
│       └── t5/
├── reranker_model.onnx                     ← MS-MARCO MiniLM reranker
├── reranker_tokenizer.json                 ← Reranker tokenizer
├── OpenClaw/                               ← OpenClaw engine state
│   ├── state/
│   │   ├── identity.json
│   │   ├── openclaw.json
│   │   └── auth-profiles.json
│   ├── workspace/
│   └── logs/
└── process_pids.json                       ← Process tracker for orphan cleanup
```

---

## 14. Data Flow Diagrams

### 14.1 Message Send Flow

```
User types message
    │
    ▼
Frontend: useChat hook
    ├── commands.save_message(conv_id, "user", content, images?, docs?, search_results?)
    │   └── INSERT INTO messages + UPDATE conversations.updated_at
    │
    ├── Optionally: commands.retrieve_context(query, chat_id, doc_ids, project_id)  
    │   └── [See RAG Retrieval Flow below]
    │
    ├── commands.chat_stream(messages, model, provider, ...) OR
    │   commands.agent_chat(messages, provider, ...)
    │   └── Calls local inference server or cloud API
    │       └── Streams tokens via Tauri event "chat_token"
    │
    └── commands.save_message(conv_id, "assistant", streamed_content, ...)
        └── INSERT INTO messages + UPDATE conversations.updated_at
```

### 14.2 Document Ingestion Flow

```
User drops file
    │
    ▼
upload_document(bytes, filename)
    ├── Save to app_data/documents/{prefix}_{filename}
    └── Return path
    │
    ▼
ingest_document(path, chat_id?, project_id?)
    ├── Read file → SHA-256 hash
    ├── Dedup check: SELECT id FROM documents WHERE hash=?
    │   └── If exists & indexed → update scope, return existing ID
    ├── INSERT INTO documents (status='processing')
    ├── Extract content (text/PDF/OCR)
    ├── Chunk content (1000 chars, 100 overlap)
    ├── For each chunk (5 parallel):
    │   ├── POST /v1/embeddings → 384-dim vector
    │   ├── INSERT INTO chunks (...) RETURNING rowid
    │   └── vector_store.add(rowid, embedding)
    ├── vector_store.save()
    └── UPDATE documents SET status='indexed'
```

### 14.3 RAG Retrieval Flow

```
retrieve_context(query, chat_id, doc_ids, project_id)
    │
    ├── Resolve project_id from conversation if needed
    │
    ├── Special case: overview query → return file list
    ├── Special case: filename in query → return full document
    │
    ├── Vector Search:
    │   ├── Embed query via /v1/embeddings
    │   ├── Search scoped indices (project/chat + global)
    │   └── Return top-150 rowids
    │
    ├── FTS Search:
    │   ├── chunks_fts MATCH "query"
    │   └── Return top-150 rowids
    │
    ├── RRF Fusion:
    │   ├── Score each rowid: 1/(60 + rank + 1) from each source
    │   └── Sort by combined score
    │
    ├── Fetch content from SQLite:
    │   ├── SELECT content, path FROM chunks JOIN documents WHERE rowid IN (...)
    │   └── Optionally filter by doc_ids
    │
    ├── Rerank (ONNX cross-encoder):
    │   ├── Score each candidate against query
    │   ├── Filter by threshold (-5.0 or -10.0)
    │   └── Return top 5
    │
    └── Emit search status events → return formatted context strings
```

### 14.4 Secret Storage Flow

```
User enters API key in Settings
    │
    ▼
Frontend → openclaw_save_{provider}_key(key)
    ├── OpenClawConfig::update_{provider}_key()
    │   ├── keychain::set_key(provider, Some(key))
    │   │   ├── Update in-memory cache
    │   │   └── flush_cache() → set_generic_password() → Keychain
    │   ├── Update in-memory OpenClawConfig field
    │   └── save_identity() → write identity.json (NO key values, only _granted flags)
    │
    └── SecretStore also available:
        ├── SecretStore::set(provider, Some(key))
        │   ├── keychain::set_key() → Keychain
        │   └── Update RwLock<HashMap>
        └── SecretStore::get(provider) → read from RwLock cache (no Keychain access)
```

---

## 15. Concurrency & Thread Safety

| Component | Synchronization | Pattern |
|-----------|----------------|---------|
| `SqlitePool` | `sqlx::SqlitePool` (internal pool) | 5 concurrent connections; WAL mode allows concurrent reads |
| `VectorStoreManager` | `Arc<Mutex<HashMap<..>>>` | Global lock on the scope→store map; per-store `Mutex<Index>` |
| `VectorStore` | `Mutex<Index>` | All index operations serialized per store |
| `SecretStore` | `RwLock<HashMap<String,String>>` | Multiple concurrent readers, exclusive writer |
| `keychain::key_cache()` | `Mutex<HashMap>` | Serialized access; separate from SecretStore's cache |
| `ConfigManager` | `Mutex<UserConfig>` | Serialized read/write; async disk flush |
| `OpenClawManager` | `tokio::sync::Mutex<Option<OpenClawConfig>>` | Async-aware mutex for config access |
| `DownloadManager` | `Arc<Mutex<HashMap>>` for cancel flags | Per-download AtomicBool cancel flags |
| `RerankerWrapper` | `Arc<Option<Reranker>>` with internal `Mutex<Session>` | Serialized ONNX inference |
| `SidecarManager` | `Mutex<SidecarState>` | Serialized sidecar process management |
| `RigManagerCache` | Custom TTL cache | Cached Rig agent instances with invalidation |

**Transaction Patterns:**
- **Explicit transactions** used for multi-statement atomicity: `edit_message`, `delete_all_history`, `delete_project`, `update_*_order`
- **Implicit transactions** (single INSERT/UPDATE): `save_message`, `create_conversation`

---

## 16. Known Limitations & Technical Debt

### Database Design

| Issue | Description | Impact |
|-------|-------------|--------|
| **JSON columns vs. normalized tables** | `images`, `attached_docs`, `web_search_results` are stored as JSON strings in `messages` | Cannot be efficiently queried/indexed; schema evolution requires JSON migration |
| **No WAL checkpoint management** | SQLite WAL checkpointing left to default auto-checkpoint at 1000 pages | Potential for large WAL file growth under heavy write load |
| **Missing indexes** | No index on `messages.conversation_id` (relies on FK) or `messages.created_at` | Pagination queries on large conversations could slow down |
| **Inconsistent timestamp units** | `conversations` use seconds, `documents`/`projects` use milliseconds | Potential confusion; requires careful handling in queries |

### Vector Store

| Issue | Description | Impact |
|-------|-------------|--------|
| **Fixed dimensions** | Hardcoded to 384 at initialization; changing requires rebuilding all indices | Cannot switch embedding models without full re-ingestion |
| **No tombstone support** | Deleting a document deletes chunks from SQLite but doesn't remove vectors from USearch | Ghost vectors accumulate until full reset; mitigated by scope deletion |
| **Lazy loading** | Integrity check on startup may report false mismatches because indices aren't loaded until first query | Misleading diagnostic output |

### Secret Storage

| Issue | Description | Impact |
|-------|-------------|--------|
| **macOS-only** | `security-framework` crate only works on macOS | Linux/Windows builds would need alternative secret storage |
| **Dual cache** | Both `keychain::key_cache()` and `SecretStore::keys` hold copies | Potential inconsistency if one is updated without the other |
| **Auth-profiles.json** | Contains plaintext API keys (generated at runtime) | Any process with filesystem access can read granted keys |

### General

| Issue | Description | Impact |
|-------|-------------|--------|
| **No backup/export** | No built-in mechanism to backup or export the database | Data loss risk if `openclaw.db` is corrupted |
| **No migration rollback** | `sqlx::migrate!` doesn't support down migrations | Schema changes are one-way |
| **Document file orphaning** | Deleting a document from the DB doesn't delete the file from `app_data/documents/` | Filesystem bloat over time; only `delete_all_history` cleans files |
| **Embedding server dependency** | RAG ingestion and retrieval both require the embedding sidecar to be running | Documents cannot be indexed or searched if the embedding server is down |

---

*This document was generated from direct source code analysis of the Scrappy codebase (commit date: 2026-02-24). All file paths, function signatures, and SQL schemas are verified against the actual implementation.*
