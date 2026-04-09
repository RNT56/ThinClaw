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
pub use embeddings::{EmbeddingProvider, MockEmbeddings, OllamaEmbeddings, OpenAiEmbeddings};
#[cfg(feature = "postgres")]
pub use repository::Repository;
pub use search::{
    RankedResult, SearchConfig, SearchResult, apply_temporal_decay, expand_query_keywords,
    mmr_rerank, reciprocal_rank_fusion,
};
pub use workspace_core::Workspace;

use std::sync::Arc;

use uuid::Uuid;

use crate::error::WorkspaceError;

/// Internal storage abstraction for Workspace.
///
/// Allows Workspace to work with either a PostgreSQL `Repository` (the original
/// path) or any `Database` trait implementation (e.g. libSQL backend).
#[derive(Clone)]
enum WorkspaceStorage {
    /// PostgreSQL-backed repository (uses connection pool directly).
    #[cfg(feature = "postgres")]
    Repo(Repository),
    /// Generic backend implementing the Database trait.
    Db(Arc<dyn crate::db::Database>),
}

impl WorkspaceStorage {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.get_document_by_path(user_id, agent_id, path).await,
            Self::Db(db) => db.get_document_by_path(user_id, agent_id, path).await,
        }
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.get_document_by_id(id).await,
            Self::Db(db) => db.get_document_by_id(id).await,
        }
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.get_or_create_document_by_path(user_id, agent_id, path)
                    .await
            }
            Self::Db(db) => {
                db.get_or_create_document_by_path(user_id, agent_id, path)
                    .await
            }
        }
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.update_document(id, content).await,
            Self::Db(db) => db.update_document(id, content).await,
        }
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.delete_document_by_path(user_id, agent_id, path).await,
            Self::Db(db) => db.delete_document_by_path(user_id, agent_id, path).await,
        }
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.list_directory(user_id, agent_id, directory).await,
            Self::Db(db) => db.list_directory(user_id, agent_id, directory).await,
        }
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.list_all_paths(user_id, agent_id).await,
            Self::Db(db) => db.list_all_paths(user_id, agent_id).await,
        }
    }

    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<(), WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.replace_chunks(document_id, chunks).await,
            Self::Db(db) => db.replace_chunks(document_id, chunks).await,
        }
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.update_chunk_embedding(chunk_id, embedding).await,
            Self::Db(db) => db.update_chunk_embedding(chunk_id, embedding).await,
        }
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.get_chunks_without_embeddings(user_id, agent_id, limit)
                    .await
            }
            Self::Db(db) => {
                db.get_chunks_without_embeddings(user_id, agent_id, limit)
                    .await
            }
        }
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.hybrid_search(user_id, agent_id, query, embedding, config)
                    .await
            }
            Self::Db(db) => {
                db.hybrid_search(user_id, agent_id, query, embedding, config)
                    .await
            }
        }
    }
}
