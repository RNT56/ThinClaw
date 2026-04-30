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

#[cfg(feature = "bedrock")]
pub use thinclaw_workspace::BedrockEmbeddings;
#[cfg(feature = "postgres")]
pub use thinclaw_workspace::Repository;
pub use thinclaw_workspace::{
    ChunkConfig, ChunkingStrategy, EmbeddingProvider, MemoryChunk, MemoryDocument, MockEmbeddings,
    OllamaEmbeddings, OpenAiEmbeddings, RankedResult, SearchConfig, SearchResult, Workspace,
    WorkspaceBackend, WorkspaceEntry, WorkspaceStore, apply_temporal_decay, chunk, chunk_document,
    expand_query_keywords, mmr_rerank, paths, reciprocal_rank_fusion,
};
