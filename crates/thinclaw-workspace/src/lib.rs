//! Storage-oriented workspace types and algorithms.

pub mod chunker;
pub mod document;
pub mod search;
pub mod store;

pub use chunker::{ChunkConfig, ChunkingStrategy, chunk, chunk_document};
pub use document::{MemoryChunk, MemoryDocument, WorkspaceEntry, paths};
pub use search::{
    RankedResult, SearchConfig, SearchResult, apply_temporal_decay, expand_query_keywords,
    mmr_rerank, reciprocal_rank_fusion,
};
pub use store::WorkspaceStore;
