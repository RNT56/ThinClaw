//! Storage-oriented workspace types and algorithms.

pub mod chunker;
pub mod citations;
pub mod document;
pub mod embeddings;
pub mod hygiene;
pub mod lancedb;
pub mod qmd;
#[cfg(feature = "postgres")]
pub mod repository;
pub mod search;
pub mod sqlite_vec;
pub mod store;
pub mod workspace_core;

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
pub use store::WorkspaceStore;
pub use workspace_core::Workspace;

use std::sync::Arc;

/// Storage backend for workspace operations.
pub type WorkspaceBackend = Arc<dyn WorkspaceStore>;
