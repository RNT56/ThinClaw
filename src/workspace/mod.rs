//! Workspace and memory system (OpenClaw-inspired).
//!
//! The workspace provides persistent memory for agents with a flexible
//! filesystem-like structure. Agents can create arbitrary markdown file
//! hierarchies that get indexed for full-text and semantic search.
//!
//! # Filesystem-like API
//!
//! ```text
//! workspace/
//! ├── README.md              <- Root runbook/index
//! ├── MEMORY.md              <- Long-term curated memory
//! ├── HEARTBEAT.md           <- Periodic checklist
//! ├── context/               <- Identity and context
//! │   ├── vision.md
//! │   └── priorities.md
//! ├── daily/                 <- Daily logs
//! │   ├── 2024-01-15.md
//! │   └── 2024-01-16.md
//! ├── projects/              <- Arbitrary structure
//! │   └── alpha/
//! │       ├── README.md
//! │       └── notes.md
//! └── ...
//! ```
//!
//! # Key Operations
//!
//! - `read(path)` - Read a file
//! - `write(path, content)` - Create or update a file
//! - `append(path, content)` - Append to a file
//! - `list(dir)` - List directory contents
//! - `delete(path)` - Delete a file
//! - `search(query)` - Full-text + semantic search across all files
//!
//! # Key Patterns
//!
//! 1. **Memory is persistence**: If you want to remember something, write it
//! 2. **Flexible structure**: Create any directory/file hierarchy you need
//! 3. **Self-documenting**: Use README.md files to describe directory structure
//! 4. **Hybrid search**: Vector similarity + BM25 full-text via RRF

mod chunker;
pub mod citations;
mod document;
mod embeddings;
pub mod hygiene;
pub mod lancedb;
pub mod qmd;
#[cfg(feature = "postgres")]
mod repository;
mod search;
pub mod sqlite_vec;
mod workspace_core;

pub use chunker::{ChunkConfig, ChunkingStrategy, chunk, chunk_document};
pub use document::{MemoryChunk, MemoryDocument, WorkspaceEntry, paths};
#[cfg(feature = "bedrock")]
pub use embeddings::BedrockEmbeddings;
pub use embeddings::{EmbeddingProvider, MockEmbeddings, OllamaEmbeddings, OpenAiEmbeddings};
#[cfg(feature = "postgres")]
pub use repository::Repository;
pub use search::{
    RankedResult, SearchConfig, SearchResult, apply_temporal_decay, expand_query_keywords,
    mmr_rerank, reciprocal_rank_fusion,
};
pub use workspace_core::Workspace;

use std::sync::Arc;

/// Storage backend for workspace operations.
///
/// Any type implementing `WorkspaceStore` can serve as a backend. Both
/// `Repository` (PostgreSQL) and `Arc<dyn Database>` compatibility paths use
/// this trait object.
pub type WorkspaceBackend = Arc<dyn crate::db::WorkspaceStore>;
