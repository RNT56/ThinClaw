//! Database repository for workspace persistence.
//!
//! All workspace data is stored in PostgreSQL:
//! - Documents in `memory_documents` table
//! - Chunks in `memory_chunks` table (with FTS and vector indexes)

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use pgvector::Vector;
use uuid::Uuid;

use thinclaw_types::error::WorkspaceError;

use thinclaw_workspace::document::{MemoryChunk, MemoryDocument, WorkspaceEntry};
use thinclaw_workspace::search::{
    RankedResult, SearchConfig, SearchResult, apply_temporal_decay, expand_query_keywords,
    mmr_rerank, reciprocal_rank_fusion,
};

/// Database repository for workspace operations.
#[derive(Clone)]
pub struct Repository {
    pool: Pool,
}

impl Repository {
    /// Create a new repository with a connection pool.
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Get a connection from the pool.
    async fn conn(&self) -> Result<deadpool_postgres::Object, WorkspaceError> {
        self.pool
            .get()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to get connection: {}", e),
            })
    }

    // ==================== Document Operations ====================

    /// Get a document by its path.
    pub async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self.conn().await?;

        let row = conn
            .query_opt(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3
                "#,
                &[&user_id, &agent_id, &path],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match row {
            Some(row) => Ok(self.row_to_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: path.to_string(),
                user_id: user_id.to_string(),
            }),
        }
    }

    /// Get a document by ID.
    pub async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self.conn().await?;

        let row = conn
            .query_opt(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents WHERE id = $1
                "#,
                &[&id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match row {
            Some(row) => Ok(self.row_to_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: "unknown".to_string(),
                user_id: "unknown".to_string(),
            }),
        }
    }

    /// Get or create a document by path.
    pub async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        // Try to get existing document first
        match self.get_document_by_path(user_id, agent_id, path).await {
            Ok(doc) => return Ok(doc),
            Err(WorkspaceError::DocumentNotFound { .. }) => {}
            Err(e) => return Err(e),
        }

        // Create new document
        let conn = self.conn().await?;
        let id = Uuid::new_v4();
        let now = Utc::now();
        let metadata = serde_json::json!({});

        conn.execute(
            r#"
            INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata, created_at, updated_at)
            VALUES ($1, $2, $3, $4, '', $5, $6, $7)
            ON CONFLICT (user_id, agent_id, path) DO NOTHING
            "#,
            &[&id, &user_id, &agent_id, &path, &metadata, &now, &now],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Insert failed: {}", e),
        })?;

        // Fetch the document (might have been created by concurrent request)
        self.get_document_by_path(user_id, agent_id, path).await
    }

    /// Update a document's content.
    pub async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE memory_documents SET content = $2, updated_at = NOW() WHERE id = $1",
            &[&id, &content],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Update failed: {}", e),
        })?;

        Ok(())
    }

    /// Delete a document by its path.
    pub async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;

        // First get the document to delete its chunks
        let doc = self.get_document_by_path(user_id, agent_id, path).await?;
        self.delete_chunks(doc.id).await?;

        // Delete the document
        conn.execute(
            r#"
            DELETE FROM memory_documents
            WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3
            "#,
            &[&user_id, &agent_id, &path],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Delete failed: {}", e),
        })?;

        Ok(())
    }

    /// List files and directories in a directory path.
    ///
    /// Returns immediate children (not recursive).
    /// Empty string lists the root directory.
    pub async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                "SELECT path, is_directory, updated_at, content_preview FROM list_workspace_files($1, $2, $3)",
                &[&user_id, &agent_id, &directory],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List directory failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .map(|row| {
                let updated_at: Option<DateTime<Utc>> = row.get("updated_at");
                WorkspaceEntry {
                    path: row.get("path"),
                    is_directory: row.get("is_directory"),
                    updated_at,
                    content_preview: row.get("content_preview"),
                }
            })
            .collect())
    }

    /// List all file paths in the workspace (flat list).
    pub async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT path FROM memory_documents
                WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2
                ORDER BY path
                "#,
                &[&user_id, &agent_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List paths failed: {}", e),
            })?;

        Ok(rows.iter().map(|row| row.get("path")).collect())
    }

    /// List all documents for a user.
    pub async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2
                ORDER BY updated_at DESC
                "#,
                &[&user_id, &agent_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        Ok(rows.iter().map(|r| self.row_to_document(r)).collect())
    }

    fn row_to_document(&self, row: &tokio_postgres::Row) -> MemoryDocument {
        MemoryDocument {
            id: row.get("id"),
            user_id: row.get("user_id"),
            agent_id: row.get("agent_id"),
            path: row.get("path"),
            content: row.get("content"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            metadata: row.get("metadata"),
        }
    }

    // ==================== Chunk Operations ====================

    /// Delete all chunks for a document.
    pub async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;

        conn.execute(
            "DELETE FROM memory_chunks WHERE document_id = $1",
            &[&document_id],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Delete failed: {}", e),
        })?;

        Ok(())
    }

    /// Insert a chunk.
    pub async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        let embedding_vec = embedding.map(|e| Vector::from(e.to_vec()));

        conn.execute(
            r#"
            INSERT INTO memory_chunks (id, document_id, chunk_index, content, embedding)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            &[&id, &document_id, &chunk_index, &content, &embedding_vec],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Insert failed: {}", e),
        })?;

        Ok(id)
    }

    /// Atomically replace all chunks for a document using a Postgres transaction.
    ///
    /// Acquires a single connection, opens a transaction, deletes all existing
    /// chunks, inserts the new set, and commits — so there is never a window
    /// where the document has zero search chunks.
    pub async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<(), WorkspaceError> {
        let mut conn = self.conn().await?;
        let tx = conn
            .transaction()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("BEGIN failed: {}", e),
            })?;

        tx.execute(
            "DELETE FROM memory_chunks WHERE document_id = $1",
            &[&document_id],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Delete failed: {}", e),
        })?;

        for (index, content, embedding) in chunks {
            let chunk_id = Uuid::new_v4();
            let embedding_vec = embedding.as_ref().map(|e| Vector::from(e.clone()));
            let content_str: &str = content;
            tx.execute(
                r#"INSERT INTO memory_chunks (id, document_id, chunk_index, content, embedding)
                   VALUES ($1, $2, $3, $4, $5)"#,
                &[&chunk_id, &document_id, index, &content_str, &embedding_vec],
            )
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("Insert failed: {}", e),
            })?;
        }

        tx.commit()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("COMMIT failed: {}", e),
            })?;

        Ok(())
    }

    /// Update a chunk's embedding.
    pub async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;
        let embedding_vec = Vector::from(embedding.to_vec());

        conn.execute(
            "UPDATE memory_chunks SET embedding = $2 WHERE id = $1",
            &[&chunk_id, &embedding_vec],
        )
        .await
        .map_err(|e| WorkspaceError::EmbeddingFailed {
            reason: format!("Update failed: {}", e),
        })?;

        Ok(())
    }

    /// Get chunks without embeddings for backfilling.
    pub async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT c.id, c.document_id, c.chunk_index, c.content, c.created_at
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
                  AND c.embedding IS NULL
                LIMIT $3
                "#,
                &[&user_id, &agent_id, &(limit as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .map(|row| MemoryChunk {
                id: row.get("id"),
                document_id: row.get("document_id"),
                chunk_index: row.get("chunk_index"),
                content: row.get("content"),
                embedding: None,
                created_at: row.get("created_at"),
            })
            .collect())
    }

    // ==================== Search Operations ====================

    /// Perform hybrid search combining FTS and vector similarity.
    ///
    /// Pipeline: query expansion → FTS + vector search → RRF fusion →
    /// temporal decay (if configured) → MMR re-ranking (if configured).
    pub async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        // Expand query with morphological variants for better FTS recall.
        let expanded_query = if config.use_fts {
            let keywords = expand_query_keywords(query);
            if keywords.is_empty() {
                query.to_string()
            } else {
                keywords.join(" | ")
            }
        } else {
            query.to_string()
        };

        let fts_results = if config.use_fts {
            self.fts_search(user_id, agent_id, &expanded_query, config.pre_fusion_limit)
                .await?
        } else {
            Vec::new()
        };

        let need_embeddings = config.enable_mmr;
        let vector_results = if config.use_vector {
            if let Some(embedding) = embedding {
                self.vector_search(
                    user_id,
                    agent_id,
                    embedding,
                    config.pre_fusion_limit,
                    need_embeddings,
                )
                .await?
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Collect timestamps and embeddings from raw results before fusion
        // (RRF discards them, so we capture them here).
        let mut doc_timestamps = std::collections::HashMap::new();
        let mut chunk_embeddings = std::collections::HashMap::new();

        for r in fts_results.iter().chain(vector_results.iter()) {
            if let Some(ts) = r.created_at {
                doc_timestamps.entry(r.document_id).or_insert(ts);
            }
            if let Some(ref emb) = r.embedding {
                chunk_embeddings
                    .entry(r.chunk_id)
                    .or_insert_with(|| emb.clone());
            }
        }

        let mut results = reciprocal_rank_fusion(fts_results, vector_results, config);

        // Apply temporal decay if configured.
        if let Some(half_life) = config.temporal_decay_half_life_days
            && !doc_timestamps.is_empty()
        {
            apply_temporal_decay(&mut results, half_life, &doc_timestamps);
        }

        // Apply MMR diversity re-ranking if configured.
        if config.enable_mmr && !chunk_embeddings.is_empty() {
            results = mmr_rerank(results, &chunk_embeddings, config.mmr_lambda, config.limit);
        }

        Ok(results)
    }

    /// Full-text search using PostgreSQL ts_rank_cd.
    ///
    /// The query may contain `|` for OR-expanded terms from `expand_query_keywords`.
    async fn fts_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RankedResult>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT c.id as chunk_id, c.document_id, d.path, c.content,
                       ts_rank_cd(c.content_tsv, plainto_tsquery('english', $3)) as rank,
                       c.created_at
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
                  AND c.content_tsv @@ plainto_tsquery('english', $3)
                ORDER BY rank DESC
                LIMIT $4
                "#,
                &[&user_id, &agent_id, &query, &(limit as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("FTS query failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, row)| RankedResult {
                chunk_id: row.get("chunk_id"),
                document_id: row.get("document_id"),
                path: row.get("path"),
                content: row.get("content"),
                rank: (i + 1) as u32,
                created_at: row.get("created_at"),
                embedding: None,
            })
            .collect())
    }

    /// Vector similarity search using pgvector cosine distance.
    ///
    /// When `include_embeddings` is true, the embedding vectors are returned
    /// in each `RankedResult` for downstream MMR re-ranking.
    async fn vector_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        embedding: &[f32],
        limit: usize,
        include_embeddings: bool,
    ) -> Result<Vec<RankedResult>, WorkspaceError> {
        let conn = self.conn().await?;
        let embedding_vec = Vector::from(embedding.to_vec());

        // When MMR is enabled we also need the raw embedding vectors.
        let query_sql = if include_embeddings {
            r#"
            SELECT c.id as chunk_id, c.document_id, d.path, c.content,
                   1 - (c.embedding <=> $3) as similarity,
                   c.created_at, c.embedding
            FROM memory_chunks c
            JOIN memory_documents d ON d.id = c.document_id
            WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
              AND c.embedding IS NOT NULL
            ORDER BY c.embedding <=> $3
            LIMIT $4
            "#
        } else {
            r#"
            SELECT c.id as chunk_id, c.document_id, d.path, c.content,
                   1 - (c.embedding <=> $3) as similarity,
                   c.created_at
            FROM memory_chunks c
            JOIN memory_documents d ON d.id = c.document_id
            WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
              AND c.embedding IS NOT NULL
            ORDER BY c.embedding <=> $3
            LIMIT $4
            "#
        };

        let rows = conn
            .query(
                query_sql,
                &[&user_id, &agent_id, &embedding_vec, &(limit as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Vector query failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let emb = if include_embeddings {
                    row.try_get::<_, Vector>("embedding")
                        .ok()
                        .map(|v| v.to_vec())
                } else {
                    None
                };
                RankedResult {
                    chunk_id: row.get("chunk_id"),
                    document_id: row.get("document_id"),
                    path: row.get("path"),
                    content: row.get("content"),
                    rank: (i + 1) as u32,
                    created_at: row.get("created_at"),
                    embedding: emb,
                }
            })
            .collect())
    }
}

#[async_trait]
impl crate::WorkspaceStore for Repository {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        Repository::get_document_by_path(self, user_id, agent_id, path).await
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        Repository::get_document_by_id(self, id).await
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        Repository::get_or_create_document_by_path(self, user_id, agent_id, path).await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        Repository::update_document(self, id, content).await
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        Repository::delete_document_by_path(self, user_id, agent_id, path).await
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        Repository::list_directory(self, user_id, agent_id, directory).await
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        Repository::list_all_paths(self, user_id, agent_id).await
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        Repository::list_documents(self, user_id, agent_id).await
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        Repository::delete_chunks(self, document_id).await
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        Repository::insert_chunk(self, document_id, chunk_index, content, embedding).await
    }

    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<(), WorkspaceError> {
        Repository::replace_chunks(self, document_id, chunks).await
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        Repository::update_chunk_embedding(self, chunk_id, embedding).await
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        Repository::get_chunks_without_embeddings(self, user_id, agent_id, limit).await
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        Repository::hybrid_search(self, user_id, agent_id, query, embedding, config).await
    }
}
