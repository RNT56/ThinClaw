# Cloud Storage Implementation

> **Status:** Design Document — Not Yet Implemented  
> **Author:** AI-assisted analysis, 2026-02-24  
> **Scope:** Full bidirectional data migration between local device storage and cloud storage providers

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Current Storage Architecture](#2-current-storage-architecture)
3. [Design Philosophy](#3-design-philosophy)
4. [Cloud Storage Providers](#4-cloud-storage-providers)
5. [Data Classification](#5-data-classification)
6. [Architecture Design](#6-architecture-design)
7. [Migration Flows](#7-migration-flows)
8. [SQLite Database Migration — Feasibility Analysis](#8-sqlite-database-migration--feasibility-analysis)
9. [Encryption & Security](#9-encryption--security)
10. [Frontend UI Design](#10-frontend-ui-design)
11. [Error Handling & Edge Cases](#11-error-handling--edge-cases)
12. [Implementation Plan](#12-implementation-plan)
13. [Open Questions & Decisions](#13-open-questions--decisions)
14. [File Access Abstraction Layer (Critical Refactor)](#14-file-access-abstraction-layer-critical-refactor)
15. [Image & File Serving in Cloud Mode](#15-image--file-serving-in-cloud-mode)
16. [OAuth Flows for Consumer Providers](#16-oauth-flows-for-consumer-providers)
17. [Tauri IPC Commands & TypeScript Bindings](#17-tauri-ipc-commands--typescript-bindings)
18. [Provider-to-Provider Switching](#18-provider-to-provider-switching)
19. [Cost Transparency & Provider Comparison](#19-cost-transparency--provider-comparison)
20. [Offline & Degraded Connectivity Behavior](#20-offline--degraded-connectivity-behavior)
21. [Sync Status UI](#21-sync-status-ui)
22. [Vector Index Rebuild Strategy](#22-vector-index-rebuild-strategy)

---

## 1. Executive Summary

Users should be able to choose between **local storage** (current default) and **cloud storage** as their single, global storage backend. This is a binary, app-wide setting — not per-project or per-chat. When a user switches from local to cloud (or vice versa), a full migration transfers ALL data. The user's device retains only a lightweight cache for performance.

**Key principles:**
- **Global toggle** — cloud vs. local is an app-wide decision, not per-entity
- **Fully reversible** — user can migrate local → cloud → local → cloud without data loss
- **Complete** — all data migrates: DB records, files, images, vector indices
- **Encrypted** — all data encrypted client-side before upload; cloud provider never sees plaintext
- **Model files are excluded** — too large, separately managed via HF Hub download system

---

## 2. Current Storage Architecture

All application data lives under `$APP_DATA_DIR` (typically `~/Library/Application Support/com.scrappy.app/`):

```
$APP_DATA_DIR/
├── openclaw.db              ← SQLite database (all relational data)
├── openclaw.db-wal          ← WAL journal (temporary, in-flight writes)
├── openclaw.db-shm          ← Shared memory file (temporary)
├── documents/               ← Ingested document files (.pdf, .md, .txt, etc.)
│   ├── {hash1}.pdf
│   └── {hash2}.md
├── images/                  ← Chat-attached images (user uploads)
│   ├── {uuid1}.png
│   └── {uuid2}.jpg
├── vectors/                 ← USearch index files (per-scope)
│   ├── global_384.usearch
│   ├── project_{id}_384.usearch
│   └── chat_{id}_384.usearch
├── previews/                ← Document preview thumbnails
│   └── {hash}.jpg
├── generated/               ← Stable Diffusion generated images
│   └── {uuid}.png
├── openclaw/                ← OpenClaw agent state
│   ├── sessions/
│   ├── soul.md
│   ├── memory.md
│   └── ...
├── reranker/                ← ONNX reranker model (~80 MB)
│   └── model.onnx
└── config/                  ← App configuration (non-secret)
    └── settings.json
```

### Storage Budget by Category

| Category | Typical Range | Growth Pattern |
|----------|--------------|----------------|
| SQLite DB (`openclaw.db`) | 5 – 100 MB | Linear with chat history |
| Document files | 10 MB – 2 GB | User-driven (uploads) |
| Chat images | 1 MB – 500 MB | Linear with image sharing |
| Generated images | 50 MB – 10 GB | Heavy with Imagine usage |
| Vector indices | 5 – 200 MB | Proportional to document count |
| Previews | 1 – 50 MB | 1 per document |
| OpenClaw state | 1 – 10 MB | Slow growth |
| Reranker model | ~80 MB | Static |
| **Total (excl. LLM models)** | **150 MB – 15 GB** | |

**Conclusion:** The majority of storage is consumed by **generated images** and **document files**. The SQLite DB is a relatively small portion. All categories should be included in the migration for completeness and simplicity.

---

## 3. Design Philosophy

### 3.1 Global Toggle — Not Per-Entity

The storage mode is a **single app-wide setting**: `local` or `cloud:{provider}`.

**Why not per-project/per-chat?**
- Users shouldn't have to think about *where* each piece of data lives
- Per-entity tracking creates complex partial states (some local, some cloud)
- Migration logic becomes exponentially harder with mixed states
- The UI stays clean: one toggle in Settings, one clear state

### 3.2 Local-First Architecture

Even in cloud mode, the app always works against local SQLite and local file paths. The cloud is a **backing store**, not a live remote database:

```
┌─────────────────────────────────────────────┐
│  App (always reads/writes local)            │
│  ┌──────────────┐  ┌───────────────────┐    │
│  │ SQLite (local)│  │ Files (local cache)│   │
│  └──────┬───────┘  └────────┬──────────┘    │
│         │                   │               │
│    ┌────▼───────────────────▼────┐          │
│    │       Sync Engine           │          │
│    │  (background upload/download)│         │
│    └────────────┬───────────────┘          │
└─────────────────┼───────────────────────────┘
                  │
         ┌────────▼────────┐
         │  Cloud Provider  │
         │  (S3 / iCloud /  │
         │   GCS / etc.)    │
         └─────────────────┘
```

**Benefits:**
- Works offline — writes go to local DB, sync happens when connection resumes
- No latency on read/write operations
- No dependency on cloud provider uptime for basic usage
- Migration is a bulk transfer, not a live connection

### 3.3 Fully Reversible

The migration flow is designed to be **idempotent and reversible**:

```
        ┌──────────────────┐
        │    Local Mode     │ ← Default
        │  (all data local) │
        └────────┬─────────┘
                 │ "Switch to Cloud" 
                 │ (upload all → delete local)
                 ▼
        ┌──────────────────┐
        │    Cloud Mode     │
        │  (cloud primary,  │
        │   local cache)    │
        └────────┬─────────┘
                 │ "Switch to Local"
                 │ (download all → delete cloud)
                 ▼
        ┌──────────────────┐
        │    Local Mode     │
        │  (all data local) │
        └──────────────────┘
```

No data loss at any step. The migration is **transactional** — if it fails partway, the original mode is preserved.

---

## 4. Cloud Storage Providers

### 4.1 Provider Taxonomy

We support providers via a **unified trait** backed by the `opendal` crate (Apache-licensed, 50+ backends, production-proven). This gives us native support for:

#### Tier 1 — Primary Targets (built-in UI support)

| Provider | Protocol | Auth Model | Notes |
|----------|----------|-----------|-------|
| **iCloud Drive** | macOS NSFileManager | System Apple ID | Zero-config for Mac users; uses ubiquity container |
| **Google Drive** | Google Drive API v3 | OAuth 2.0 | Most users have a Google account |
| **Dropbox** | Dropbox API v2 | OAuth 2.0 | Widely installed on desktops |
| **OneDrive** | Microsoft Graph API | OAuth 2.0 | Common with Microsoft 365 users |

#### Tier 2 — S3-Compatible Object Storage (advanced users)

| Provider | Protocol | Auth Model | Notes |
|----------|----------|-----------|-------|
| **AWS S3** | S3 API | Access Key + Secret | Industry standard |
| **Google Cloud Storage** | S3-compat or JSON API | Service Account Key / OAuth | GCS XML API is S3-compatible |
| **Azure Blob** | Azure Blob API | Connection String / SAS Token | Microsoft's object store |
| **Cloudflare R2** | S3-compat | Access Key + Secret | Zero egress fees — cheapest for retrieval |
| **Backblaze B2** | S3-compat | Application Key | Very cheap per-GB storage |
| **Wasabi** | S3-compat | Access Key + Secret | No egress fees, hot storage |
| **DigitalOcean Spaces** | S3-compat | Access Key + Secret | Simple, affordable |
| **Hetzner Storage Box** | WebDAV / SFTP | Username + Password | EU data privacy compliant |

#### Tier 3 — Self-Hosted (power users)

| Provider | Protocol | Auth Model | Notes |
|----------|----------|-----------|-------|
| **MinIO** | S3-compat | Access Key + Secret | Self-hosted S3 |
| **WebDAV** | WebDAV/HTTP | Username + Password | Nextcloud, ownCloud, Synology |
| **SFTP** | SSH | Key / Password | Any Linux server |

### 4.2 Recommended Crate: `opendal`

[Apache OpenDAL](https://opendal.apache.org/) (`opendal` crate, v0.55+) provides:

- **50+ storage backends** via a single `Operator` API
- S3, GCS, Azure, Dropbox, Google Drive, OneDrive, WebDAV, SFTP, iCloud, etc.
- Async Rust with `tokio` (already our runtime)
- Automatic retry, buffered I/O, progress callbacks
- Apache 2.0 licensed
- Production-proven (used by Databend, GreptimeDB, Vector)

```rust
// Example: same code works with any backend
let op = Operator::via_iter(Scheme::S3, configs)?;
op.write("scrappy/archive/2026/db_snapshot.enc", data).await?;
let data = op.read("scrappy/archive/2026/db_snapshot.enc").await?;
```

### 4.3 iCloud via Native APIs

iCloud Drive is special — it doesn't use a network API. Instead, macOS manages sync transparently via `NSFileManager.ubiquityContainerURL`. Files placed in the ubiquity container are automatically synced by the OS.

```rust
// Conceptual — uses objc2 bindings
let container_url = NSFileManager::defaultManager()
    .URLForUbiquityContainerIdentifier(None)?;
let scrappy_dir = container_url.appendingPathComponent("Scrappy");
// Writing a file here → macOS syncs to iCloud automatically
std::fs::write(scrappy_dir.join("archive.enc"), &data)?;
```

**Advantages:** Zero network code, OS handles retries/conflicts, free 5 GB  
**Limitations:** macOS only, requires Apple Developer entitlement, user must be signed into iCloud

---

## 5. Data Classification

### 5.1 What Migrates

| Data | Storage | Migrated? | How |
|------|---------|-----------|-----|
| SQLite database | `openclaw.db` | ✅ YES | VACUUM INTO snapshot → encrypt → upload |
| Document files | `documents/` | ✅ YES | Per-file encrypt → upload |
| Chat images | `images/` | ✅ YES | Per-file encrypt → upload |
| Generated images | `generated/` | ✅ YES | Per-file encrypt → upload |
| Vector indices | `vectors/` | ✅ YES | Per-file upload (rebuilt on restore if corrupt) |
| Preview thumbnails | `previews/` | ✅ YES | Per-file upload (small, convenient) |
| OpenClaw state | `openclaw/` | ✅ YES | Directory tar → encrypt → upload |
| Reranker model | `reranker/` | ⚠️ SKIP | Re-downloaded from HF Hub on demand (~80 MB) |

### 5.2 What Does NOT Migrate

| Data | Reason |
|------|--------|
| **LLM model files** (`~/.cache/huggingface/`) | Multi-GB, separately managed, re-downloadable |
| **Reranker ONNX model** | Re-downloaded automatically on first use |
| **App configuration** (`settings.json`) | Device-specific (GPU settings, paths, etc.) |
| **API keys** (macOS Keychain) | Never leave the device — keys are re-entered on new devices |
| **SQLite WAL/SHM files** | Temporary; checkpointed into main DB before snapshot |
| **Process tracker state** | Runtime-only, ephemeral |

---

## 6. Architecture Design

### 6.1 Core Components

```
backend/src/
├── cloud/
│   ├── mod.rs              ← Public API: migrate_to_cloud(), migrate_to_local()
│   ├── provider.rs         ← CloudProvider trait + factory
│   ├── providers/
│   │   ├── s3.rs           ← S3-compat (AWS, R2, B2, Wasabi, DO, Hetzner, MinIO)
│   │   ├── icloud.rs       ← macOS NSFileManager-based
│   │   ├── gdrive.rs       ← Google Drive API
│   │   ├── dropbox.rs      ← Dropbox API
│   │   ├── onedrive.rs     ← Microsoft Graph
│   │   ├── webdav.rs       ← WebDAV (Nextcloud, Synology)
│   │   └── sftp.rs         ← SFTP (any server)
│   ├── manifest.rs         ← Archive manifest (JSON index of all files)
│   ├── encryption.rs       ← AES-256-GCM envelope encryption
│   ├── snapshot.rs         ← SQLite VACUUM INTO + WAL checkpoint
│   └── progress.rs         ← Progress reporting via Tauri events
```

### 6.2 CloudProvider Trait

```rust
#[async_trait]
pub trait CloudProvider: Send + Sync {
    /// Human-readable name for UI
    fn name(&self) -> &str;
    
    /// Test connectivity and authentication
    async fn test_connection(&self) -> Result<CloudStatus, CloudError>;
    
    /// Upload a blob with a given key
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError>;
    
    /// Download a blob by key
    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError>;
    
    /// Delete a blob by key
    async fn delete(&self, key: &str) -> Result<(), CloudError>;
    
    /// List all keys with a given prefix
    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError>;
    
    /// Get total storage used (bytes)
    async fn usage(&self) -> Result<u64, CloudError>;
    
    /// Maximum single-file upload size (for chunking decisions)
    fn max_upload_size(&self) -> u64;
}

pub struct CloudEntry {
    pub key: String,
    pub size: u64,
    pub last_modified: i64,
    pub checksum: Option<String>,
}

pub struct CloudStatus {
    pub connected: bool,
    pub storage_used: u64,
    pub storage_available: Option<u64>,  // None if unlimited
    pub provider_name: String,
}
```

### 6.3 Archive Manifest

Every cloud archive includes a JSON manifest as the index:

```json
{
    "version": 1,
    "app_version": "0.1.0",
    "schema_version": 12,
    "created_at_ms": 1740422400000,
    "encryption": {
        "algorithm": "AES-256-GCM",
        "key_derivation": "HKDF-SHA256",
        "key_id": "scrappy-cloud-key-2026-02-24"
    },
    "files": [
        {
            "key": "db/openclaw.db.enc",
            "original_path": "openclaw.db",
            "size_bytes": 15728640,
            "encrypted_size_bytes": 15728672,
            "sha256": "a1b2c3d4...",
            "type": "database"
        },
        {
            "key": "documents/abc123.pdf.enc",
            "original_path": "documents/abc123.pdf",
            "size_bytes": 2097152,
            "encrypted_size_bytes": 2097184,
            "sha256": "e5f6g7h8...",
            "type": "document"
        }
    ],
    "statistics": {
        "total_files": 247,
        "total_size_bytes": 1073741824,
        "conversations": 45,
        "messages": 3200,
        "documents": 23,
        "generated_images": 180
    }
}
```

The manifest serves as:
- An integrity check (SHA-256 per file)
- A progress indicator (X of Y files transferred)
- A schema version guard (prevents restoring into incompatible app version)
- Human-readable summary of what's stored

### 6.4 Storage Schema Changes

```sql
-- New migration: track cloud storage state

-- App-wide storage mode
CREATE TABLE IF NOT EXISTS cloud_config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Keys: 'mode' ('local'|'cloud'), 'provider' ('s3'|'icloud'|...),
--        'provider_config' (encrypted JSON), 'last_sync_at' (ms timestamp),
--        'manifest_key' (cloud path to manifest)

-- Migration history (for auditing and rollback)
CREATE TABLE IF NOT EXISTS cloud_migrations (
    id          TEXT PRIMARY KEY,
    direction   TEXT NOT NULL,       -- 'to_cloud' | 'to_local'
    provider    TEXT NOT NULL,
    started_at  INTEGER NOT NULL,    -- ms
    completed_at INTEGER,            -- ms, NULL if in-progress or failed
    files_total INTEGER NOT NULL,
    files_done  INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL,
    bytes_done  INTEGER NOT NULL DEFAULT 0,
    status      TEXT NOT NULL DEFAULT 'in_progress',  -- 'in_progress'|'completed'|'failed'|'cancelled'
    error       TEXT                 -- NULL on success
);
```

---

## 7. Migration Flows

### 7.1 Local → Cloud Migration

```
User clicks "Move to Cloud" in Settings
│
├── 1. PRE-FLIGHT CHECKS
│   ├── Verify cloud provider credentials (test_connection())
│   ├── Estimate total data size
│   ├── Check cloud storage quota (if applicable)
│   ├── Warn user: "This will upload X GB to {provider}. Continue?"
│   └── Ensure no active chat generation / document ingestion
│
├── 2. PREPARE DATABASE SNAPSHOT
│   ├── PRAGMA wal_checkpoint(TRUNCATE)  — flush WAL to main DB
│   ├── VACUUM INTO 'openclaw_snapshot.db'  — atomic, consistent copy
│   │   (App continues using original DB during upload)
│   └── Record schema migration version in manifest
│
├── 3. ENCRYPT & UPLOAD
│   ├── Generate archive encryption key (AES-256-GCM)
│   ├── Store key in macOS Keychain: "com.scrappy.cloud-key"
│   ├── For each file (DB snapshot, documents, images, vectors, openclaw state):
│   │   ├── Read file
│   │   ├── Encrypt with AES-256-GCM (per-file nonce)
│   │   ├── Upload to cloud: scrappy/{user_id}/{category}/{filename}.enc
│   │   ├── Verify upload (ETag / checksum)
│   │   ├── Update progress: emit Tauri event "cloud_migration_progress"
│   │   └── Record in manifest
│   └── Upload manifest.json.enc (encrypted manifest)
│
├── 4. VERIFY CLOUD ARCHIVE
│   ├── Download manifest back
│   ├── Spot-check 3 random files (download → decrypt → verify SHA-256)
│   └── If any check fails → ABORT, keep local data, report error
│
├── 5. SWITCH MODE
│   ├── UPDATE cloud_config SET value='cloud' WHERE key='mode'
│   ├── Store provider config in cloud_config
│   └── Record successful migration in cloud_migrations
│
├── 6. CLEAN UP LOCAL (DEFERRED)
│   ├── Delete: documents/*, images/*, generated/*, vectors/*, previews/*
│   ├── Keep: openclaw.db (with cloud_config tables), config/, reranker/
│   ├── Keep: lightweight local cache directory for recently-accessed files
│   └── Log freed space
│
└── 7. CONFIRM
    ├── Show success: "Saved X GB. Data now stored in {provider}."
    └── Offer: "Undo migration" (within 24h, before cloud cleanup timer)
```

### 7.2 Cloud → Local Migration

```
User clicks "Move to Local" in Settings
│
├── 1. PRE-FLIGHT CHECKS
│   ├── Verify cloud connection
│   ├── Download and verify manifest
│   ├── Check local disk space (manifest.statistics.total_size_bytes)
│   ├── Warn user: "This will download X GB. Continue?"
│   └── Retrieve encryption key from Keychain
│
├── 2. DOWNLOAD & DECRYPT
│   ├── For each file in manifest.files:
│   │   ├── Download from cloud
│   │   ├── Decrypt with AES-256-GCM
│   │   ├── Verify SHA-256 checksum
│   │   ├── Write to original local path
│   │   └── Update progress event
│   └── Download DB snapshot last (most critical)
│
├── 3. RESTORE DATABASE
│   ├── Close current SQLite pool
│   ├── Back up current openclaw.db → openclaw.db.pre_restore
│   ├── Replace with downloaded snapshot
│   ├── Run any pending migrations (if app version is newer than snapshot)
│   │   sqlx::migrate!("./migrations").run(&pool)
│   ├── Re-open pool
│   ├── Verify: SELECT count(*) FROM conversations (sanity check)
│   └── If verification fails → restore from backup → ABORT
│
├── 4. REBUILD VECTOR INDICES (if needed)
│   ├── If vector files downloaded successfully → use as-is
│   ├── If corrupt or missing → trigger re-index from chunks table
│   │   (Chunks contain the text; embeddings can be recomputed)
│   └── This is a graceful fallback, not an error
│
├── 5. SWITCH MODE
│   ├── UPDATE cloud_config SET value='local' WHERE key='mode'
│   └── Record successful migration
│
├── 6. CLEAN UP CLOUD (OPTIONAL, USER-CHOICE)
│   ├── Option A: Keep cloud copy as backup (recommended)
│   ├── Option B: Delete all cloud data immediately
│   └── Option C: Delete cloud data after 30 days
│
└── 7. CONFIRM
    └── Show success: "All data restored locally. Using X GB."
```

### 7.3 Ongoing Sync (Cloud Mode Active)

While in cloud mode, new data (new messages, new images, etc.) needs to reach the cloud. Two approaches:

#### Approach A: Periodic Batch Sync (Recommended for MVP)

```
Every 5 minutes (configurable):
├── Check if DB has changed (file modification time)
├── If changed:
│   ├── VACUUM INTO snapshot → encrypt → upload (overwrites previous)
│   └── Upload any new files in documents/, images/, generated/
└── If not: skip
```

**Pros:** Simple, reliable, minimal code  
**Cons:** Up to 5-minute data loss window on catastrophic failure  

#### Approach B: Write-Through (Future Enhancement)

Every write operation also queues an upload. More complex but near-zero data loss.

---

## 8. SQLite Database Migration — Feasibility Analysis

### 8.1 Can the DB Be Migrated Safely? — YES ✅

SQLite provides robust tools for producing consistent snapshots:

| Mechanism | How It Works | Safety |
|-----------|-------------|--------|
| `VACUUM INTO 'path'` | Creates an atomic, standalone copy of the entire DB | ✅ No locks needed on source, copy is always consistent |
| `PRAGMA wal_checkpoint(TRUNCATE)` | Flushes WAL to main DB file, removes WAL | ✅ Ensures all committed data is in the main file |
| `.backup` API | Online backup with read-lock coordination | ✅ Production-grade, handles concurrent reads |

**Key point:** `VACUUM INTO` creates a *complete, self-contained* copy without holding locks on the source database. The app can continue serving requests during the snapshot. This is the recommended approach.

### 8.2 Size Considerations

| Scenario | Messages | Documents | DB Size | Upload Time (10 Mbps) |
|----------|----------|-----------|---------|----------------------|
| Light user | 500 | 5 | ~5 MB | < 1 second |
| Moderate user | 5,000 | 50 | ~25 MB | ~20 seconds |
| Heavy user | 50,000 | 200 | ~100 MB | ~80 seconds |
| Extreme user | 200,000 | 1,000 | ~500 MB | ~7 minutes |

Even the extreme case (500 MB) is fast enough for a one-time migration. The periodic sync only uploads a diff-sized snapshot (the full DB), which is acceptable at these sizes.

### 8.3 Schema Version Compatibility

**Risk:** User migrates to cloud on v0.1.0, later updates app to v0.2.0 (new migration), then tries to restore the cloud DB on a v0.1.0 device.

**Mitigation:**
1. The manifest records `schema_version` (the migration count)
2. On restore, `sqlx::migrate!` runs any missing migrations automatically (forward-compatible)
3. On downgrade: if the downloaded DB has a *higher* schema version than the app, show an error: "Please update the app before restoring this backup"
4. Schema version is checked BEFORE closing the current DB pool

### 8.4 Conclusion

Including the SQLite DB in the migration flow is:
- **Safe** — `VACUUM INTO` provides atomic, consistent snapshots
- **Fast** — even large DBs are < 500 MB, well within reasonable upload times
- **Simple** — one file transfer, no table-by-table sync needed
- **Compatible** — `sqlx::migrate!` handles forward schema evolution automatically
- **Reversible** — restore is just: replace file, run migrations, reopen pool

**Recommendation:** Include the full SQLite database in both migration directions. The complexity cost is minimal, and excluding it would create a confusing split where files are in the cloud but chat history is local-only.

---

## 9. Encryption & Security

### 9.1 Threat Model

| Threat | Mitigation |
|--------|-----------|
| Cloud provider reads user data | Client-side encryption (AES-256-GCM) |
| Cloud provider breach | Data is encrypted; key never leaves device |
| Man-in-the-middle | TLS for all transfers + client-side encryption |
| Key loss | Warn user prominently; optionally allow key export |
| Device theft | Keychain is protected by device password / biometrics |

### 9.2 Encryption Architecture

```
                    ┌──────────────────────────┐
                    │    macOS Keychain         │
                    │  "com.scrappy.cloud-key"  │
                    │  ┌────────────────────┐   │
                    │  │ Master Key (256-bit)│   │
                    │  └────────┬───────────┘   │
                    └───────────┼───────────────┘
                                │
                    ┌───────────▼───────────┐
                    │  HKDF-SHA256 derive   │
                    │  per-file subkeys     │
                    └───────────┬───────────┘
                                │
              ┌─────────────────┼─────────────────┐
              │                 │                  │
    ┌─────────▼──────┐ ┌──────▼───────┐ ┌────────▼───────┐
    │ File Key #1     │ │ File Key #2  │ │ File Key #N    │
    │ AES-256-GCM     │ │ AES-256-GCM  │ │ AES-256-GCM    │
    │ Random nonce    │ │ Random nonce │ │ Random nonce   │
    └────────────────┘ └──────────────┘ └────────────────┘
```

- **Master Key:** 256-bit random key, generated once, stored in macOS Keychain
- **Per-file keys:** Derived via HKDF-SHA256 from master key + file path (deterministic → same file always gets same derived key → enables incremental sync without re-encrypting unchanged files)
- **Nonces:** Random 96-bit per encryption operation
- **Crate:** `aes-gcm` (already widely used in the Rust ecosystem, AEAD)

### 9.3 Encrypted File Format

```
┌──────────────────────────────────────────┐
│ Header (16 bytes)                        │
│  ├── Magic bytes: "SCRY" (4 bytes)       │
│  ├── Version: u16 (2 bytes)              │
│  ├── Reserved: 10 bytes                  │
├──────────────────────────────────────────┤
│ Nonce (12 bytes)                         │
├──────────────────────────────────────────┤
│ Ciphertext (variable)                    │
│  (AES-256-GCM encrypted payload)         │
├──────────────────────────────────────────┤
│ Auth Tag (16 bytes)                      │
│  (GCM authentication tag)                │
└──────────────────────────────────────────┘
```

### 9.4 Key Management

- **First cloud setup:** Generate master key → store in Keychain
- **Key display:** Settings shows a "Recovery Key" (base64-encoded master key) that the user should save
- **Key import:** If restoring on a new device, user enters the recovery key
- **Key rotation:** "Rotate Encryption Key" button → re-encrypts all cloud data with new key

---

## 10. Frontend UI Design

### 10.1 Settings > Storage Page

```
┌─────────────────────────────────────────────────────────┐
│  ⚙️ Storage                                             │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Storage Mode                                           │
│  ┌─────────────────────────────────────────────────┐    │
│  │  ◉ Local Storage (current)                      │    │
│  │    All data stored on this device.               │    │
│  │    Using 2.4 GB of 256 GB                       │    │
│  │                                                  │    │
│  │  ○ Cloud Storage                                 │    │
│  │    Data encrypted and stored in the cloud.       │    │
│  │    Free up device storage.                       │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  Storage Breakdown                                      │
│  ┌─────────────────────────────────────────────────┐    │
│  │  Generated Images    ████████████████░░  1.8 GB  │    │
│  │  Documents           ████████░░░░░░░░░  420 MB   │    │
│  │  Chat Images         ███░░░░░░░░░░░░░░   95 MB   │    │
│  │  Database            ██░░░░░░░░░░░░░░░   45 MB   │    │
│  │  Vector Indices      █░░░░░░░░░░░░░░░░   22 MB   │    │
│  │  Other               ░░░░░░░░░░░░░░░░░    8 MB   │    │
│  │                              Total:     2.4 GB   │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ── When Cloud is selected: ──────────────────────────  │
│                                                         │
│  Cloud Provider                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  ☁️ iCloud Drive          [Connected ✓]         │    │
│  │  📁 Google Drive          [Connect]              │    │
│  │  📦 Dropbox               [Connect]              │    │
│  │  💼 OneDrive              [Connect]              │    │
│  │  🔧 S3-Compatible         [Configure]            │    │
│  │  🌐 WebDAV                [Configure]            │    │
│  │  🖥️ SFTP                  [Configure]            │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  🔐 Recovery Key                                        │
│  ┌─────────────────────────────────────────────────┐    │
│  │  Your encryption key protects your cloud data.   │    │
│  │  Save it securely — it cannot be recovered.      │    │
│  │                                                  │    │
│  │  [Show Recovery Key]   [Copy]   [Rotate Key]     │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  [Migrate to Cloud →]                                   │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### 10.2 Migration Progress Dialog

```
┌─────────────────────────────────────────────────────────┐
│  ☁️ Migrating to Cloud Storage                          │
│                                                         │
│  ████████████████████░░░░░░░░░░  67%                   │
│                                                         │
│  Uploading generated images...                          │
│  142 of 213 files  •  1.2 GB of 1.8 GB                 │
│                                                         │
│  ⏱️ Estimated time remaining: 3 minutes                 │
│  📶 Upload speed: 12.4 MB/s                             │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  ✅ Database snapshot              45 MB         │    │
│  │  ✅ Documents (23 files)          420 MB         │    │
│  │  ⏳ Generated images (142/213)    1.2 / 1.8 GB  │    │
│  │  ⬜ Chat images (95 files)         95 MB         │    │
│  │  ⬜ Vector indices                  22 MB         │    │
│  │  ⬜ Other data                       8 MB         │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ⚠️ Do not close the app during migration.              │
│                                                         │
│                                        [Cancel]         │
└─────────────────────────────────────────────────────────┘
```

---

## 11. Error Handling & Edge Cases

### 11.1 Migration Failure Recovery

| Failure Point | Recovery Strategy |
|---------------|-------------------|
| Upload fails midway | Resume from last successful file (manifest tracks progress) |
| Download fails midway | Resume from last successful file; local data preserved |
| DB snapshot corrupt | Retry VACUUM INTO; if persistent, report I/O error |
| Encryption key lost | User must enter recovery key; if lost, data is irrecoverable (by design) |
| Cloud quota exceeded | Pause migration, inform user, suggest upgrading plan or cleaning up |
| Network timeout | Automatic retry with exponential backoff (3 attempts per file) |
| App crashes during migration | On next launch, detect `cloud_migrations.status = 'in_progress'`, offer resume |
| Schema version mismatch | Prevent restore with higher-version DB; auto-migrate lower-version DB |

### 11.2 Concurrent Access

| Scenario | Handling |
|----------|---------|
| User sends message during upload | Message saved to local DB; captured in next sync cycle |
| User generates image during upload | Image saved locally; added to upload queue |
| User closes app during migration | Migration paused; resumed on next launch |
| Two devices on same cloud account | NOT SUPPORTED in v1 — one device owns the archive. Multi-device sync is a v2 feature. |

### 11.3 Data Integrity Guarantees

1. **Checksums:** SHA-256 for every file, stored in manifest
2. **Verification pass:** After upload, spot-check random files
3. **Atomic mode switch:** `cloud_config.mode` only updated AFTER full verification
4. **Backup before restore:** Current DB backed up before overwrite
5. **Idempotent operations:** Re-running migration skips already-transferred files

---

## 12. Implementation Plan

### Phase 1: Foundation (Week 1–2)

| Task | File(s) | Priority |
|------|---------|----------|
| Add `opendal` to Cargo.toml dependencies | `Cargo.toml` | P0 |
| Create `backend/src/cloud/mod.rs` module structure | `cloud/` | P0 |
| Implement `CloudProvider` trait | `cloud/provider.rs` | P0 |
| Implement S3-compatible provider (covers AWS, R2, B2, Wasabi, DO, MinIO) | `cloud/providers/s3.rs` | P0 |
| Implement encryption layer (AES-256-GCM + HKDF) | `cloud/encryption.rs` | P0 |
| Implement SQLite snapshot (`VACUUM INTO` + WAL checkpoint) | `cloud/snapshot.rs` | P0 |
| Create `cloud_config` and `cloud_migrations` schema migration | `migrations/` | P0 |
| Implement archive manifest generation and parsing | `cloud/manifest.rs` | P0 |

### Phase 2: Migration Engine (Week 2–3)

| Task | File(s) | Priority |
|------|---------|----------|
| Implement `migrate_to_cloud()` — full local→cloud flow | `cloud/mod.rs` | P0 |
| Implement `migrate_to_local()` — full cloud→local flow | `cloud/mod.rs` | P0 |
| Implement progress reporting via Tauri events | `cloud/progress.rs` | P0 |
| Implement migration resume (detect `in_progress` on launch) | `cloud/mod.rs` | P1 |
| Register Tauri commands: `cloud_status`, `cloud_migrate`, `cloud_test` | `lib.rs` | P0 |
| Implement periodic sync (background task for cloud mode) | `cloud/sync.rs` | P1 |

### Phase 3: Additional Providers (Week 3–4)

| Task | File(s) | Priority |
|------|---------|----------|
| Implement iCloud provider (macOS NSFileManager) | `cloud/providers/icloud.rs` | P1 |
| Implement Google Drive provider | `cloud/providers/gdrive.rs` | P1 |
| Implement Dropbox provider | `cloud/providers/dropbox.rs` | P2 |
| Implement OneDrive provider | `cloud/providers/onedrive.rs` | P2 |
| Implement WebDAV provider (Nextcloud, Synology) | `cloud/providers/webdav.rs` | P2 |
| Implement SFTP provider | `cloud/providers/sftp.rs` | P3 |

### Phase 4: Frontend (Week 4–5)

| Task | File(s) | Priority |
|------|---------|----------|
| Create Storage settings page | `frontend/src/components/settings/StorageTab.tsx` | P0 |
| Create storage breakdown visualization | `frontend/src/components/settings/StorageBreakdown.tsx` | P1 |
| Create cloud provider selection UI | `frontend/src/components/settings/CloudProviderPicker.tsx` | P0 |
| Create S3 configuration form (endpoint, bucket, keys) | `frontend/src/components/settings/S3ConfigForm.tsx` | P0 |
| Create OAuth flow for Google Drive / Dropbox / OneDrive | Multiple | P1 |
| Create migration progress dialog | `frontend/src/components/settings/MigrationProgress.tsx` | P0 |
| Create recovery key display / import UI | `frontend/src/components/settings/RecoveryKey.tsx` | P1 |
| Add cloud badge indicators throughout app (optional polish) | Various | P3 |

### Phase 5: Testing & Polish (Week 5–6)

| Task | Priority |
|------|----------|
| Unit tests for encryption/decryption roundtrip | P0 |
| Unit tests for manifest generation/parsing | P0 |
| Integration test: local → cloud → local roundtrip (with MinIO in Docker) | P0 |
| Integration test: schema migration on restore (old DB + new app) | P1 |
| Integration test: resume after interrupted migration | P1 |
| Load test: migration with 10 GB dataset | P2 |
| UX test: error messages and edge case flows | P1 |

---

## 13. Open Questions & Decisions

### Must Decide Before Implementation

| # | Question | Options | Recommendation |
|---|----------|---------|----------------|
| 1 | **Sync frequency in cloud mode** | On every write / Every 5 min / Manual only | Every 5 min (simple, reliable) |
| 2 | **Multi-device support** | v1: single device / v2: multi-device sync | Single device in v1; multi-device is a major feature |
| 3 | **Local cache size in cloud mode** | No cache / 500 MB / Configurable | 500 MB LRU cache for recently accessed files |
| 4 | **File chunking for large uploads** | Single file / 100 MB chunks / Provider default | 100 MB chunks (handles large generated images) |
| 5 | **Recovery key UX** | Show once at setup / Show anytime / Allow export | Show anytime in settings + copy button |
| 6 | **OAuth for consumer providers** | In-app browser / System browser redirect | System browser (more trustworthy to users) |
| 7 | **Compression before encryption** | None / zstd / gzip | zstd (SQLite and text compress 5–10×) |
| 8 | **Delete cloud data on "Move to Local"?** | Always / Ask user / Keep as backup | Ask user (3 options: keep, delete now, delete in 30d) |

### Future Enhancements (Post-v1)

- **Multi-device sync** — CRDT-based or last-write-wins merge for concurrent edits
- **Selective sync** — keep some data local, some cloud (contradicts current design, but users may request)
- **Streaming API** — upload/download large files without holding in memory
- **Deduplicated storage** — content-addressed storage to avoid duplicate uploads
- **Cross-platform** — Windows (DPAPI for key storage), Linux (Secret Service API)
- **Version history** — keep last N snapshots for point-in-time recovery

---

## 14. File Access Abstraction Layer (Critical Refactor)

### 14.1 The Core Problem

The codebase currently has **~30 direct `std::fs` / `tokio::fs` call sites** that read and write files using hardcoded paths relative to `app_data_dir`. In cloud mode, these files may not exist locally. Every file access point must go through an abstraction.

**Current call sites that break in cloud mode:**

| File | Operation | What Breaks |
|------|-----------|-------------|
| `images.rs:19` | `app_data_dir.join("images")` — upload_image | Writes go to local only, not synced |
| `images.rs:62` | `images_dir.join(format!("{}.png", id))` — get_image_path | File may not exist locally |
| `images.rs:98` | `tokio::fs::read(path)` — load_image_as_base64 | File may be cloud-only |
| `imagine.rs:243` | `app_data_dir.join("images")` — save generated image | Same as above |
| `imagine.rs:565` | `std::fs::remove_file(&file_path)` — delete image | Must also delete from cloud |
| `rag.rs:36` | `app_data_dir.join("documents")` — document upload | Must sync to cloud |
| `history.rs:406-412` | `docs_dir`, `images_dir` cleanup | Must clean cloud too |
| `projects.rs:241-247` | `std::fs::remove_file(&path)` — delete doc | Must also delete from cloud |
| `openclaw/commands/sessions.rs:642-760` | Various `read_to_string` / `write` for session files | Session state must sync |

### 14.2 The FileStore Abstraction

Instead of refactoring every call site to use the `CloudProvider` directly, introduce a **`FileStore`** managed state that wraps all file I/O:

```rust
/// Managed as Tauri state: app.manage(FileStore::new(...))
pub struct FileStore {
    /// Local root directory (always exists, used as cache in cloud mode)
    local_root: PathBuf,
    
    /// If in cloud mode, the cloud provider + sync engine
    cloud: Option<CloudBackend>,
    
    /// LRU cache tracking which files are available locally
    cache: Mutex<LruCache<String, CacheEntry>>,
}

struct CloudBackend {
    provider: Box<dyn CloudProvider>,
    config: CloudConfig,
    encryption: ArchiveEncryption,
}

impl FileStore {
    /// Write a file. In local mode: writes to disk. In cloud mode: writes
    /// to local cache AND queues for cloud upload.
    pub async fn write(&self, relative_path: &str, data: &[u8]) -> Result<PathBuf, FileStoreError>;
    
    /// Read a file. In local mode: reads from disk. In cloud mode: reads
    /// from local cache if available, otherwise downloads from cloud,
    /// decrypts, caches, and returns.
    pub async fn read(&self, relative_path: &str) -> Result<Vec<u8>, FileStoreError>;
    
    /// Get the local path for a file, ensuring it exists locally.
    /// In cloud mode, this may trigger a download. Returns the local
    /// path that can be used with convertFileSrc() / asset://.
    pub async fn ensure_local(&self, relative_path: &str) -> Result<PathBuf, FileStoreError>;
    
    /// Delete a file from both local and cloud.
    pub async fn delete(&self, relative_path: &str) -> Result<(), FileStoreError>;
    
    /// Check if a file exists (in either local cache or cloud).
    pub async fn exists(&self, relative_path: &str) -> Result<bool, FileStoreError>;
    
    /// Get the absolute local path (may or may not exist on disk).
    /// Used for constructing asset:// URLs when we know the file is cached.
    pub fn local_path(&self, relative_path: &str) -> PathBuf;
    
    /// List all files under a prefix.
    pub async fn list(&self, prefix: &str) -> Result<Vec<String>, FileStoreError>;
}
```

### 14.3 Migration Path — Incremental Refactor

Rather than a big-bang rewrite, migrate each module incrementally:

```
Phase A: Add FileStore with local-only implementation (no behavior change)
    ├── FileStore::write() → std::fs::write()
    ├── FileStore::read()  → std::fs::read()
    └── Tests pass, zero behavior change

Phase B: Update each module to use FileStore instead of direct fs
    ├── images.rs     → FileStore (6 call sites)
    ├── imagine.rs    → FileStore (4 call sites)
    ├── rag.rs        → FileStore (2 call sites)
    ├── projects.rs   → FileStore (2 call sites)
    ├── history.rs    → FileStore (2 call sites)
    └── openclaw/     → FileStore (8 call sites)

Phase C: Add cloud backend to FileStore
    ├── write() → local + queue upload
    ├── read()  → local cache || cloud download
    └── Cloud mode now works
```

### 14.4 Relative Path Convention

Currently, the DB stores **absolute paths** like `/Users/alice/Library/Application Support/com.scrappy.app/images/abc.png`. These break when migrating to a different machine.

**Required migration:** Convert all stored paths to **relative paths** (relative to `app_data_dir`):

```sql
-- New migration: normalize to relative paths
UPDATE generated_images
SET file_path = REPLACE(file_path, ?, 'images/')
WHERE file_path LIKE ?;
-- Parameter 1: user's app_data_dir + '/images/'
-- Parameter 2: user's app_data_dir + '/images/%'

UPDATE documents
SET path = REPLACE(path, ?, 'documents/')
WHERE path LIKE ?;
```

This must happen **before** the first cloud migration, as a prerequisite migration.

---

## 15. Image & File Serving in Cloud Mode

### 15.1 The Asset Protocol Problem

The frontend uses Tauri's `asset://` protocol and `convertFileSrc()` to display images:

```typescript
// ImagineGallery.tsx:111
const assetUrl = convertFileSrc(image.filePath);

// imagine.ts:108
return `asset://localhost/${encodeURIComponent(filePath)}`;

// MessageBubble.tsx:118
const assetUrl = convertFileSrc(res.data);
```

The `asset://` protocol serves files directly from the local filesystem. In cloud mode, files may not exist locally yet.

### 15.2 Solution: Lazy Download with Placeholder

```
Frontend requests image via convertFileSrc(path)
    │
    ├── If file exists locally → Tauri serves it immediately ✅
    │
    └── If file NOT local (cloud mode) → Tauri returns 404
        │
        ├── Frontend catches load error on <img> element
        ├── Displays blur placeholder / skeleton
        ├── Calls Tauri command: cloud_ensure_local(relative_path)
        │   └── Backend: FileStore::ensure_local() → downloads, decrypts, caches
        ├── Backend emits event: "file_cached" { path }
        └── Frontend retries the asset URL → now succeeds ✅
```

### 15.3 Frontend Image Component Wrapper

```typescript
// components/CloudImage.tsx — drop-in replacement for <img>
function CloudImage({ filePath, alt, ...props }: CloudImageProps) {
    const [src, setSrc] = useState(convertFileSrc(filePath));
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState(false);
    const { storageMode } = useStorageContext();

    const handleError = useCallback(async () => {
        if (storageMode !== 'cloud' || loading) return;
        setLoading(true);
        setError(false);
        try {
            await commands.cloudEnsureLocal(filePath);
            // Force re-render with cache-busted URL
            setSrc(convertFileSrc(filePath) + `?t=${Date.now()}`);
        } catch (e) {
            setError(true);
        } finally {
            setLoading(false);
        }
    }, [filePath, storageMode, loading]);

    if (loading) return <Skeleton className="w-full h-full" />;
    if (error) return <CloudOfflineIcon />;
    return <img src={src} alt={alt} onError={handleError} {...props} />;
}
```

### 15.4 Where CloudImage Needs to Replace `<img>`

| Component | Current | Files |
|-----------|---------|-------|
| `ImagineGallery.tsx` | `<img src={convertFileSrc(img.filePath)} />` | 2 instances |
| `ImagineGeneration.tsx` | `<img src={convertFileSrc(img.filePath)} />` | 2 instances |
| `MessageBubble.tsx` | `<img src={convertFileSrc(res.data)} />` | 1 instance |
| `ImagineView.tsx` | `convertFileSrc(image.filePath)` | 1 instance |
| `ChatProvider.tsx` | `convertFileSrc(result.filePath)` | 1 instance |

---

## 16. OAuth Flows for Consumer Providers

### 16.1 The Challenge

Google Drive, Dropbox, and OneDrive require OAuth 2.0 authorization. A desktop Tauri app can't use a simple redirect URI like `http://localhost:3000/callback` because:
- The app doesn't run a web server
- Users may have firewalls blocking localhost
- Deep links (`scrappy://callback`) require OS-level URL scheme registration

### 16.2 Recommended Pattern: Loopback + PKCE

```
1. App starts a temporary local HTTP server on a random port (e.g., 127.0.0.1:49152)
2. App opens system browser to provider's OAuth page with:
   - redirect_uri = http://127.0.0.1:49152/callback
   - code_challenge (PKCE)
   - state (CSRF protection)
3. User authorizes in browser
4. Provider redirects to http://127.0.0.1:49152/callback?code=xxx
5. Local server receives the code, shuts down
6. App exchanges code for tokens using PKCE verifier
7. Tokens stored in macOS Keychain (encrypted)
```

**Why PKCE?** Public clients (desktop apps) can't securely store a client secret. PKCE (Proof Key for Code Exchange) eliminates the need for one.

### 16.3 Token Management

```rust
pub struct OAuthTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: i64,  // ms timestamp
    provider: String,
}

impl OAuthTokens {
    /// Check if token needs refresh (5-minute buffer)
    fn needs_refresh(&self) -> bool;
    
    /// Refresh using the refresh token
    async fn refresh(&mut self, provider_config: &ProviderConfig) -> Result<(), OAuthError>;
}
```

Tokens stored in Keychain under `com.scrappy.oauth.{provider}`.

### 16.4 Per-Provider OAuth Details

| Provider | Auth URL | Token URL | Scopes | Client ID Source |
|----------|----------|-----------|--------|-----------------|
| Google Drive | `accounts.google.com/o/oauth2/v2/auth` | `oauth2.googleapis.com/token` | `drive.file` | Embedded in app (public client) |
| Dropbox | `www.dropbox.com/oauth2/authorize` | `api.dropboxapi.com/oauth2/token` | `files.content.write files.content.read` | Embedded in app |
| OneDrive | `login.microsoftonline.com/.../authorize` | `login.microsoftonline.com/.../token` | `Files.ReadWrite.All offline_access` | Embedded in app |

**Note:** Scrappy must register developer apps with each provider to get client IDs. These are public (embedded in the binary) — security comes from PKCE, not secret.

---

## 17. Tauri IPC Commands & TypeScript Bindings

### 17.1 Backend Commands

```rust
// ═══════════════════════════════════════════════════════════
// Cloud Storage — Tauri Commands
// ═══════════════════════════════════════════════════════════

/// Get current storage mode and status
#[tauri::command]
#[specta::specta]
pub async fn cloud_get_status(/* deps */) -> Result<CloudStatusResponse, String>;

/// Get storage usage breakdown (local disk usage per category)
#[tauri::command]
#[specta::specta]
pub async fn cloud_get_storage_breakdown(/* deps */) -> Result<StorageBreakdown, String>;

/// Test a cloud provider connection with given credentials
#[tauri::command]
#[specta::specta]
pub async fn cloud_test_connection(
    provider: String,
    config: serde_json::Value,
) -> Result<CloudTestResult, String>;

/// Start migration to cloud (returns immediately, runs in background)
#[tauri::command]
#[specta::specta]
pub async fn cloud_migrate_to_cloud(
    provider: String,
    config: serde_json::Value,
) -> Result<String, String>;  // returns migration_id

/// Start migration back to local (returns immediately, runs in background)
#[tauri::command]
#[specta::specta]
pub async fn cloud_migrate_to_local() -> Result<String, String>;  // returns migration_id

/// Cancel an in-progress migration
#[tauri::command]
#[specta::specta]
pub async fn cloud_cancel_migration(migration_id: String) -> Result<(), String>;

/// Get migration progress (polled by frontend during migration)
#[tauri::command]
#[specta::specta]
pub async fn cloud_get_migration_progress(
    migration_id: String,
) -> Result<MigrationProgress, String>;

/// Ensure a file is available locally (downloads from cloud if needed)
#[tauri::command]
#[specta::specta]
pub async fn cloud_ensure_local(relative_path: String) -> Result<String, String>;

/// Get the recovery key (base64-encoded master encryption key)
#[tauri::command]
#[specta::specta]
pub async fn cloud_get_recovery_key() -> Result<String, String>;

/// Import a recovery key (for restoring on a new device)
#[tauri::command]
#[specta::specta]
pub async fn cloud_import_recovery_key(key_base64: String) -> Result<(), String>;

/// Start OAuth flow for a consumer provider (opens system browser)
#[tauri::command]
#[specta::specta]
pub async fn cloud_start_oauth(provider: String) -> Result<(), String>;

/// Force a sync cycle (for manual "Sync Now" button)
#[tauri::command]
#[specta::specta]
pub async fn cloud_sync_now() -> Result<(), String>;
```

### 17.2 Tauri Events (Backend → Frontend)

```rust
// Emitted during migration progress
#[derive(Serialize, Type)]
struct CloudMigrationProgress {
    migration_id: String,
    phase: String,           // "database" | "documents" | "images" | "vectors" | "openclaw"
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    current_file: Option<String>,
    speed_bytes_per_sec: u64,
    eta_seconds: Option<u64>,
}

// Emitted when a cloud file becomes locally available
#[derive(Serialize, Type)]
struct CloudFileCached {
    relative_path: String,
    local_path: String,
}

// Emitted on sync status changes
#[derive(Serialize, Type)]
struct CloudSyncStatus {
    state: String,           // "idle" | "syncing" | "error" | "offline"
    last_sync_at: Option<i64>,
    pending_files: u64,
    error: Option<String>,
}

// Emitted after OAuth callback
#[derive(Serialize, Type)]
struct CloudOAuthResult {
    provider: String,
    success: bool,
    error: Option<String>,
}
```

### 17.3 Generated TypeScript Types (via specta)

```typescript
// Auto-generated in bindings.ts
export type CloudStatusResponse = {
    mode: 'local' | 'cloud';
    provider: string | null;
    connected: boolean;
    storage_used_cloud: number | null;
    storage_available_cloud: number | null;
    last_sync_at: number | null;
    encryption_key_exists: boolean;
};

export type StorageBreakdown = {
    total_bytes: number;
    categories: StorageCategory[];
};

export type StorageCategory = {
    name: string;        // "Generated Images", "Documents", etc.
    bytes: number;
    file_count: number;
};

export type MigrationProgress = {
    migration_id: string;
    direction: 'to_cloud' | 'to_local';
    status: 'in_progress' | 'completed' | 'failed' | 'cancelled';
    phases: MigrationPhase[];
    overall_percent: number;
    speed_bytes_per_sec: number;
    eta_seconds: number | null;
    error: string | null;
};

export type MigrationPhase = {
    name: string;
    status: 'pending' | 'in_progress' | 'completed' | 'failed';
    files_done: number;
    files_total: number;
    bytes_done: number;
    bytes_total: number;
};
```

---

## 18. Provider-to-Provider Switching

### 18.1 Can a User Switch Cloud Providers Without Going Local First?

**Yes**, but it requires a cloud-to-cloud transfer. The flow:

```
User is on AWS S3, wants to switch to Cloudflare R2
│
├── 1. Configure new provider (R2 credentials)
├── 2. Test R2 connection
├── 3. Download all from S3 → local cache (temporary)
├── 4. Upload all from local cache → R2
├── 5. Verify R2 archive
├── 6. Update cloud_config to point to R2
├── 7. (Optional) Delete S3 data
└── 8. Clear local cache (keep configured cache size)
```

This is internally `cloud→local + local→cloud` but presented to the user as a single "Switch Provider" operation. The local cache is temporary and cleaned up after.

### 18.2 UI for Provider Switching

When already in cloud mode, the Storage settings page shows:
```
Current Provider: AWS S3  [Switch Provider ▾]
```

Clicking "Switch Provider" opens the provider picker. After selecting and configuring the new provider, a migration dialog shows transfer progress.

---

## 19. Cost Transparency & Provider Comparison

### 19.1 Estimated Monthly Costs

The Storage settings page should display estimated monthly costs based on the user's data size:

| Provider | Storage (per GB/mo) | Egress (per GB) | Est. Cost (5 GB) | Est. Cost (50 GB) |
|----------|-------------------|-----------------|--------------------|---------------------|
| iCloud Drive | Free (5 GB), $0.99/50 GB | Free | **Free** | **$0.99** |
| Google Drive | Free (15 GB), $1.99/100 GB | Free | **Free** | **$1.99** |
| Dropbox | Free (2 GB), $9.99/2 TB | Free | **$9.99** | **$9.99** |
| OneDrive | Free (5 GB), $1.99/100 GB | Free | **Free** | **$1.99** |
| Cloudflare R2 | $0.015/GB | **Free** | **$0.08** | **$0.75** |
| Backblaze B2 | $0.005/GB | $0.01/GB | **$0.03** | **$0.25** |
| Wasabi | $0.0069/GB | Free | **$0.03** | **$0.35** |
| AWS S3 Standard | $0.023/GB | $0.09/GB | **$0.12** | **$1.15** |
| Hetzner Storage Box | €3.45/1 TB flat | N/A | **€3.45** | **€3.45** |

### 19.2 UI Cost Indicator

```
Cloud Provider: Cloudflare R2
Estimated cost: $0.75/month for your 50 GB
```

This uses the `StorageBreakdown` data + provider pricing table (hardcoded, updated with app releases).

---

## 20. Offline & Degraded Connectivity Behavior

### 20.1 Cloud Mode Offline Behavior

| Scenario | User Experience |
|----------|----------------|
| App opens while offline | Works normally with local cache. Banner: "☁️ Offline — changes will sync when connected" |
| User sends a message offline | Saved to local DB. Queued for sync. |
| User generates an image offline | Saved locally. Queued for sync. |
| User views an image NOT in cache | Placeholder with cloud icon: "This image is stored in the cloud. Connect to view." |
| User views a cached image offline | Works normally — served from local cache. |
| Connection restored | Automatic sync triggered. Banner: "☁️ Syncing..." |
| Sync fails repeatedly | After 3 retries: "☁️ Sync error — [Retry] [Details]" |

### 20.2 macOS App Nap Considerations

macOS will suspend background activity for idle apps (App Nap). The sync timer must:
- Use `NSProcessInfo.beginActivity(options: .userInitiated)` during active sync
- Handle timer fire-after-sleep gracefully (sync immediately on wake if overdue)
- Not prevent App Nap when idle (no forced `Background` mode)

### 20.3 Network Quality Detection

```rust
enum SyncStrategy {
    FullSync,           // Wi-Fi / Ethernet — sync everything
    MeteredSync,        // Cellular hotspot — sync DB only, defer large files
    DeferredSync,       // Very slow — queue everything, sync on better connection
}
```

Use `SCNetworkReachability` (macOS) to detect network type and adjust strategy.

---

## 21. Sync Status UI

### 21.1 Persistent Status Indicator

When in cloud mode, a small indicator in the app's status bar or sidebar shows sync state:

```
┌──┐
│☁️│ Synced 2 min ago          ← idle, all synced
└──┘

┌──┐
│🔄│ Syncing (3 files)...      ← active sync
└──┘

┌──┐
│⚠️│ Sync error — Click to fix  ← error state
└──┘

┌──┐
│📡│ Offline — queued 12 changes ← offline
└──┘
```

### 21.2 Implementation

```typescript
// hooks/use-cloud-status.ts
function useCloudStatus() {
    const [status, setStatus] = useState<CloudSyncStatus | null>(null);
    
    useEffect(() => {
        // Poll cloud_get_status every 30 seconds
        const interval = setInterval(async () => {
            const result = await commands.cloudGetStatus();
            setStatus(result);
        }, 30_000);
        
        // Also listen for real-time sync events
        const unlisten = listen<CloudSyncStatus>('cloud_sync_status', (e) => {
            setStatus(e.payload);
        });
        
        return () => {
            clearInterval(interval);
            unlisten.then(f => f());
        };
    }, []);
    
    return status;
}
```

---

## 22. Vector Index Rebuild Strategy

### 22.1 The Problem

When restoring from cloud, vector indices (`.usearch` files) may be:
- **Downloaded and valid** → use as-is ✅
- **Downloaded but corrupt** → need rebuild
- **Not downloaded** (if skipped to save bandwidth) → need rebuild

Rebuilding requires the **embedding server** to be running, which requires a downloaded embedding model. This creates a dependency chain.

### 22.2 Rebuild Flow

```
Vector index missing or corrupt after restore
│
├── Check: Is embedding model downloaded?
│   ├── YES → Start embedding server, re-index all chunks
│   │         (chunks table has full text data, no data loss)
│   └── NO → Show notification:
│            "Vector search is unavailable. Download an embedding
│             model in Settings > Models to enable semantic search."
│            (Chat still works — just no RAG retrieval)
│
└── Mark index as "pending_rebuild" in cloud_config
    └── On next embedding model load → trigger rebuild
```

### 22.3 Optimization: Skip Vector Upload

Vector indices can be **regenerated** from the chunks table (which is in the SQLite DB). We could skip uploading them entirely to save cloud storage and bandwidth:

**Savings:** 5–200 MB per archive  
**Cost:** Re-indexing takes 1–5 minutes on restore (depending on chunk count)  
**Recommendation:** Make it configurable. Default: upload vectors (faster restore). Option: "Optimize storage — rebuild vectors on restore."

---

## Dependencies

### New Rust Crates Needed

| Crate | Purpose | Size Impact |
|-------|---------|-------------|
| `opendal` | Unified cloud storage access (S3, GCS, Azure, WebDAV, SFTP, etc.) | ~2 MB compiled |
| `aes-gcm` | AES-256-GCM encryption | ~100 KB (already transitive dep of `ed25519-dalek`) |
| `hkdf` | Key derivation for per-file subkeys | ~50 KB |
| `zstd` | Compression before encryption (optional) | ~1 MB compiled |

### Existing Crates Already Available

| Crate | Usage |
|-------|-------|
| `sha2` | SHA-256 checksums (already in Cargo.toml) |
| `base64` | Recovery key encoding (already in Cargo.toml) |
| `serde_json` | Manifest serialization (already in Cargo.toml) |
| `tokio` | Async runtime (already in Cargo.toml) |
| `security-framework` | macOS Keychain for master key (already in Cargo.toml) |
| `reqwest` | OAuth token exchange (already in Cargo.toml) |
