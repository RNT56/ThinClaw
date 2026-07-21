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
        const MAX_SEARCH_QUERY_BYTES: usize = 32 * 1024;
        if query.trim().is_empty() || query.len() > MAX_SEARCH_QUERY_BYTES || query.contains('\0') {
            return Err(WorkspaceError::SearchFailed {
                reason: format!(
                    "search query must be non-empty, contain no NUL byte, and be at most {MAX_SEARCH_QUERY_BYTES} bytes"
                ),
            });
        }
        let config = config
            .validate_and_normalize()
            .map_err(|reason| WorkspaceError::SearchFailed { reason })?;

        // Repair a bounded number of durable dirty-index markers before
        // querying. Writes never become non-durable merely because embedding
        // or indexing failed; the next search heals them opportunistically.
        if let Err(error) = self.repair_dirty_indexes(16, &config.path_prefixes).await {
            tracing::warn!(%error, "Failed to repair dirty memory indexes before search");
        }

        // Generate embedding for semantic search if provider available
        let embedding = if let Some(ref provider) = self.embeddings {
            match provider.embed(query).await {
                Ok(embedding) => Some(embedding),
                Err(error) if config.use_fts => {
                    tracing::warn!(%error, "Query embedding failed; continuing with FTS recall");
                    None
                }
                Err(error) => {
                    return Err(WorkspaceError::EmbeddingFailed {
                        reason: error.to_string(),
                    });
                }
            }
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
        const MAX_CAS_RETRIES: usize = 4;
        for attempt in 0..MAX_CAS_RETRIES {
            let doc = self.storage.get_document_by_id(document_id).await?;
            let raw_chunks = chunk(&doc.content, ChunkConfig::default());
            let mut prepared: Vec<(i32, String, Option<Vec<f32>>)> =
                Vec::with_capacity(raw_chunks.len());
            for (index, content) in raw_chunks.into_iter().enumerate() {
                let embedding = if let Some(ref provider) = self.embeddings {
                    match provider.embed(&content).await {
                        Ok(embedding) => Some(embedding),
                        Err(error) => {
                            tracing::warn!(%error, "Failed to generate memory chunk embedding");
                            None
                        }
                    }
                } else {
                    None
                };
                prepared.push((index as i32, content, embedding));
            }

            if self
                .storage
                .replace_chunks_if_current(document_id, &doc.content, &prepared)
                .await?
            {
                return Ok(());
            }
            tracing::debug!(
                %document_id,
                attempt = attempt + 1,
                "Memory document changed during indexing; retrying current content"
            );
        }
        Err(WorkspaceError::ChunkingFailed {
            reason: format!(
                "document {document_id} changed during {MAX_CAS_RETRIES} indexing attempts"
            ),
        })
    }

    async fn repair_dirty_indexes(
        &self,
        limit: usize,
        path_prefixes: &[String],
    ) -> Result<usize, WorkspaceError> {
        let documents = self
            .storage
            .list_documents(&self.user_id, self.agent_id)
            .await?;
        let mut repaired = 0;
        for document in documents
            .into_iter()
            .filter(|document| {
                let path_allowed = path_prefixes.is_empty()
                    || path_prefixes.iter().any(|prefix| {
                        document.path == *prefix || document.path.starts_with(&format!("{prefix}/"))
                    });
                path_allowed
                    && document
                        .metadata
                        .get("index_dirty")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
            })
            .take(limit)
        {
            // Bound work by attempts, not successes. A persistent embedding or
            // storage failure must not make every search retry the entire
            // dirty corpus before serving any recall.
            match self.reindex_document(document.id).await {
                Ok(()) => repaired += 1,
                Err(error) => tracing::warn!(
                    document_id = %document.id,
                    path = %document.path,
                    %error,
                    "Dirty memory index repair failed"
                ),
            }
        }
        Ok(repaired)
    }

    /// Generate embeddings for chunks that don't have them yet.
    ///
    /// This is useful for backfilling embeddings after enabling the provider.
    pub async fn backfill_embeddings(&self) -> Result<usize, WorkspaceError> {
        let Some(ref provider) = self.embeddings else {
            return Ok(0);
        };

        const BATCH_SIZE: usize = 100;
        const MAX_SCAN: usize = 10_000;
        let mut attempted = std::collections::HashSet::new();
        let mut count = 0;
        loop {
            let scan_limit = (attempted.len() + BATCH_SIZE).min(MAX_SCAN);
            if scan_limit <= attempted.len() {
                break;
            }
            let chunks = self
                .storage
                .get_chunks_without_embeddings(&self.user_id, self.agent_id, scan_limit)
                .await?;
            let exhausted = chunks.len() < scan_limit;
            let pending = chunks
                .into_iter()
                .filter(|chunk| !attempted.contains(&chunk.id))
                .collect::<Vec<_>>();
            if pending.is_empty() {
                break;
            }
            for chunk in pending {
                attempted.insert(chunk.id);
                match provider.embed(&chunk.content).await {
                    Ok(embedding) => {
                        self.storage
                            .update_chunk_embedding(chunk.id, &embedding)
                            .await?;
                        count += 1;
                    }
                    Err(error) => {
                        tracing::warn!(chunk_id = %chunk.id, %error, "Failed to backfill chunk embedding");
                    }
                }
            }
            if exhausted {
                break;
            }
        }
        Ok(count)
    }
}
