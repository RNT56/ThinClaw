//! Hybrid search and document indexing on [`Workspace`].
//!
//! Owns query execution (FTS + semantic via RRF), per-document re-indexing
//! (chunk + embed + atomic chunk replacement), and embedding backfill.

use uuid::Uuid;

use thinclaw_types::error::WorkspaceError;

use super::Workspace;
use crate::chunker::{ChunkConfig, chunk};
use crate::search::{SearchConfig, SearchResult};

impl Workspace {
    // ==================== Search ====================

    /// Hybrid search across all memory documents.
    ///
    /// Combines full-text search (BM25) with semantic search (vector similarity)
    /// using Reciprocal Rank Fusion (RRF).
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        self.search_with_config(query, SearchConfig::default().with_limit(limit))
            .await
    }

    /// Search with custom configuration.
    pub async fn search_with_config(
        &self,
        query: &str,
        config: SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        // Generate embedding for semantic search if provider available
        let embedding = if let Some(ref provider) = self.embeddings {
            Some(
                provider
                    .embed(query)
                    .await
                    .map_err(|e| WorkspaceError::EmbeddingFailed {
                        reason: e.to_string(),
                    })?,
            )
        } else {
            None
        };

        self.storage
            .hybrid_search(
                &self.user_id,
                self.agent_id,
                query,
                embedding.as_deref(),
                &config,
            )
            .await
    }

    // ==================== Indexing ====================

    /// Re-index a document (chunk and generate embeddings).
    ///
    /// Chunk counts and embeddings are computed first. The old index is then
    /// atomically replaced with the new one via `storage.replace_chunks`, which
    /// wraps the delete + insert in a single BEGIN/COMMIT on libSQL so there is
    /// never a window where the document has zero search chunks.
    pub(super) async fn reindex_document(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        // Get the document content
        let doc = self.storage.get_document_by_id(document_id).await?;

        // Chunk the content
        let raw_chunks = chunk(&doc.content, ChunkConfig::default());

        // Build (index, content, embedding) tuples — generate embeddings first so
        // the expensive work happens before we touch the DB index at all.
        let mut prepared: Vec<(i32, String, Option<Vec<f32>>)> =
            Vec::with_capacity(raw_chunks.len());
        for (index, content) in raw_chunks.into_iter().enumerate() {
            let embedding = if let Some(ref provider) = self.embeddings {
                match provider.embed(&content).await {
                    Ok(emb) => Some(emb),
                    Err(e) => {
                        tracing::warn!("Failed to generate embedding: {}", e);
                        None
                    }
                }
            } else {
                None
            };
            prepared.push((index as i32, content, embedding));
        }

        // Atomically swap old chunks for new ones (single transaction on libSQL,
        // fallback sequential delete+insert on Postgres).
        self.storage.replace_chunks(document_id, &prepared).await?;

        Ok(())
    }

    /// Generate embeddings for chunks that don't have them yet.
    ///
    /// This is useful for backfilling embeddings after enabling the provider.
    pub async fn backfill_embeddings(&self) -> Result<usize, WorkspaceError> {
        let Some(ref provider) = self.embeddings else {
            return Ok(0);
        };

        let chunks = self
            .storage
            .get_chunks_without_embeddings(&self.user_id, self.agent_id, 100)
            .await?;

        let mut count = 0;
        for chunk in chunks {
            match provider.embed(&chunk.content).await {
                Ok(embedding) => {
                    self.storage
                        .update_chunk_embedding(chunk.id, &embedding)
                        .await?;
                    count += 1;
                }
                Err(e) => {
                    tracing::warn!("Failed to embed chunk {}: {}", chunk.id, e);
                }
            }
        }

        Ok(count)
    }
}
