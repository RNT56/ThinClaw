//! Workspace-related WorkspaceStore implementation for LibSqlBackend.

use std::collections::HashMap;

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_opt_ts, get_text, get_ts,
    row_to_memory_document,
};
use crate::db::WorkspaceStore;
use crate::error::WorkspaceError;
use crate::workspace::{
    MemoryChunk, MemoryDocument, RankedResult, SearchConfig, SearchResult, WorkspaceEntry,
    apply_temporal_decay, expand_query_keywords, mmr_rerank, reciprocal_rank_fusion,
};

use chrono::Utc;

const LIBSQL_VECTOR_DIM: usize = 1536;

fn serialize_libsql_embedding(embedding: &[f32]) -> (Option<Vec<u8>>, Vec<u8>, i64) {
    let canonical = embedding
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect::<Vec<u8>>();
    let indexed = if embedding.len() == LIBSQL_VECTOR_DIM {
        Some(canonical.clone())
    } else {
        tracing::debug!(
            configured_dimension = embedding.len(),
            indexed_dimension = LIBSQL_VECTOR_DIM,
            "Storing non-1536 embedding in canonical libSQL payload and skipping vector index column"
        );
        None
    };

    (indexed, canonical, embedding.len() as i64)
}

fn deserialize_libsql_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(std::mem::size_of::<f32>()) {
        return None;
    }

    Some(
        bytes
            .chunks_exact(std::mem::size_of::<f32>())
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

fn cosine_similarity(query: &[f32], candidate: &[f32]) -> Option<f32> {
    if query.len() != candidate.len() || query.is_empty() {
        return None;
    }

    let mut dot = 0.0f32;
    let mut query_norm = 0.0f32;
    let mut candidate_norm = 0.0f32;

    for (q, c) in query.iter().zip(candidate.iter()) {
        dot += q * c;
        query_norm += q * q;
        candidate_norm += c * c;
    }

    let denom = query_norm.sqrt() * candidate_norm.sqrt();
    if denom <= f32::EPSILON {
        return None;
    }

    Some(dot / denom)
}

#[async_trait]
impl WorkspaceStore for LibSqlBackend {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2 AND path = ?3
                "#,
                params![user_id, agent_id_str.as_deref(), path],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })? {
            Some(row) => Ok(row_to_memory_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: path.to_string(),
                user_id: user_id.to_string(),
            }),
        }
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents WHERE id = ?1
                "#,
                params![id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })? {
            Some(row) => Ok(row_to_memory_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: "unknown".to_string(),
                user_id: "unknown".to_string(),
            }),
        }
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        // Try get
        match self.get_document_by_path(user_id, agent_id, path).await {
            Ok(doc) => return Ok(doc),
            Err(WorkspaceError::DocumentNotFound { .. }) => {}
            Err(e) => return Err(e),
        }

        // Create
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let id = Uuid::new_v4();
        let agent_id_str = agent_id.map(|id| id.to_string());
        conn.execute(
            r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, ?3, ?4, '', '{}')
                ON CONFLICT (user_id, agent_id, path) DO NOTHING
                "#,
            params![id.to_string(), user_id, agent_id_str.as_deref(), path],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Insert failed: {}", e),
        })?;

        self.get_document_by_path(user_id, agent_id, path).await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE memory_documents SET content = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.to_string(), content, now],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Update failed: {}", e),
        })?;
        Ok(())
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        let doc = self.get_document_by_path(user_id, agent_id, path).await?;
        self.delete_chunks(doc.id).await?;

        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        conn.execute(
            "DELETE FROM memory_documents WHERE user_id = ?1 AND agent_id IS ?2 AND path = ?3",
            params![user_id, agent_id_str.as_deref(), path],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Delete failed: {}", e),
        })?;
        Ok(())
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let dir = if !directory.is_empty() && !directory.ends_with('/') {
            format!("{}/", directory)
        } else {
            directory.to_string()
        };

        let agent_id_str = agent_id.map(|id| id.to_string());
        let pattern = if dir.is_empty() {
            "%".to_string()
        } else {
            format!("{}%", dir)
        };

        let mut rows = conn
            .query(
                r#"
                SELECT path, updated_at, substr(content, 1, 200) as content_preview
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2
                  AND (?3 = '%' OR path LIKE ?3)
                ORDER BY path
                "#,
                params![user_id, agent_id_str.as_deref(), pattern],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List directory failed: {}", e),
            })?;

        let mut entries_map: HashMap<String, WorkspaceEntry> = HashMap::new();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            let full_path = get_text(&row, 0);
            let updated_at = get_opt_ts(&row, 1);
            let content_preview = get_opt_text(&row, 2);

            let relative = if dir.is_empty() {
                &full_path
            } else if let Some(stripped) = full_path.strip_prefix(&dir) {
                stripped
            } else {
                continue;
            };

            let child_name = if let Some(slash_pos) = relative.find('/') {
                &relative[..slash_pos]
            } else {
                relative
            };

            if child_name.is_empty() {
                continue;
            }

            let is_dir = relative.contains('/');
            let entry_path = if dir.is_empty() {
                child_name.to_string()
            } else {
                format!("{}{}", dir, child_name)
            };

            entries_map
                .entry(child_name.to_string())
                .and_modify(|e| {
                    if is_dir {
                        e.is_directory = true;
                        e.content_preview = None;
                    }
                    if let (Some(existing), Some(new)) = (&e.updated_at, &updated_at)
                        && new > existing
                    {
                        e.updated_at = Some(*new);
                    }
                })
                .or_insert(WorkspaceEntry {
                    path: entry_path,
                    is_directory: is_dir,
                    updated_at,
                    content_preview: if is_dir { None } else { content_preview },
                });
        }

        let mut entries: Vec<WorkspaceEntry> = entries_map.into_values().collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                "SELECT path FROM memory_documents WHERE user_id = ?1 AND agent_id IS ?2 ORDER BY path",
                params![user_id, agent_id_str.as_deref()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List paths failed: {}", e),
            })?;

        let mut paths = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            paths.push(get_text(&row, 0));
        }
        Ok(paths)
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2
                ORDER BY updated_at DESC
                "#,
                params![user_id, agent_id_str.as_deref()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        let mut docs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            docs.push(row_to_memory_document(&row));
        }
        Ok(docs)
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "DELETE FROM memory_chunks WHERE document_id = ?1",
            params![document_id.to_string()],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Delete failed: {}", e),
        })?;
        Ok(())
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: e.to_string(),
            })?;
        let id = Uuid::new_v4();
        let (indexed_embedding, canonical_embedding, embedding_dim) = embedding
            .map(serialize_libsql_embedding)
            .map_or((None, None, None), |(indexed, canonical, dim)| {
                (indexed, Some(canonical), Some(dim))
            });

        conn.execute(
            r#"
                INSERT INTO memory_chunks (
                    id, document_id, chunk_index, content, embedding, embedding_blob, embedding_dim
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
            params![
                id.to_string(),
                document_id.to_string(),
                chunk_index as i64,
                content,
                indexed_embedding.map(libsql::Value::Blob),
                canonical_embedding.map(libsql::Value::Blob),
                embedding_dim,
            ],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Insert failed: {}", e),
        })?;
        Ok(id)
    }

    /// Atomically replace all chunks for a document using a single connection
    /// and an explicit BEGIN / COMMIT transaction.
    ///
    /// This prevents the split-brain state where old chunks are deleted but
    /// new ones have not yet been written (which would make the document
    /// invisible to search until the next reindex attempt).
    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: e.to_string(),
            })?;

        // Begin transaction
        conn.execute("BEGIN", ())
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("BEGIN failed: {}", e),
            })?;

        // Delete existing chunks
        let del_result = conn
            .execute(
                "DELETE FROM memory_chunks WHERE document_id = ?1",
                params![document_id.to_string()],
            )
            .await;

        if let Err(e) = del_result {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(WorkspaceError::ChunkingFailed {
                reason: format!("Delete failed: {}", e),
            });
        }

        // Insert new chunks
        for (index, content, embedding) in chunks {
            let chunk_id = Uuid::new_v4();
            let (indexed_embedding, canonical_embedding, embedding_dim) = embedding
                .as_ref()
                .map(|e| serialize_libsql_embedding(e))
                .map_or((None, None, None), |(indexed, canonical, dim)| {
                    (indexed, Some(canonical), Some(dim))
                });

            let ins_result = conn
                .execute(
                    r#"INSERT INTO memory_chunks (
                           id, document_id, chunk_index, content, embedding, embedding_blob, embedding_dim
                       )
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                    params![
                        chunk_id.to_string(),
                        document_id.to_string(),
                        *index as i64,
                        content.as_str(),
                        indexed_embedding.map(libsql::Value::Blob),
                        canonical_embedding.map(libsql::Value::Blob),
                        embedding_dim,
                    ],
                )
                .await;

            if let Err(e) = ins_result {
                let _ = conn.execute("ROLLBACK", ()).await;
                return Err(WorkspaceError::ChunkingFailed {
                    reason: format!("Insert failed: {}", e),
                });
            }
        }

        // Commit
        conn.execute("COMMIT", ())
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("COMMIT failed: {}", e),
            })?;

        Ok(())
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::EmbeddingFailed {
                reason: e.to_string(),
            })?;
        let (indexed_embedding, canonical_embedding, embedding_dim) =
            serialize_libsql_embedding(embedding);

        conn.execute(
            r#"
                UPDATE memory_chunks
                SET embedding = ?2,
                    embedding_blob = ?3,
                    embedding_dim = ?4
                WHERE id = ?1
            "#,
            params![
                chunk_id.to_string(),
                indexed_embedding.map(libsql::Value::Blob),
                libsql::Value::Blob(canonical_embedding),
                embedding_dim,
            ],
        )
        .await
        .map_err(|e| WorkspaceError::EmbeddingFailed {
            reason: format!("Update failed: {}", e),
        })?;
        Ok(())
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT c.id, c.document_id, c.chunk_index, c.content, c.created_at
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = ?1 AND d.agent_id IS ?2
                  AND c.embedding_blob IS NULL
                LIMIT ?3
                "#,
                params![user_id, agent_id_str.as_deref(), limit as i64],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        let mut chunks = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            chunks.push(MemoryChunk {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                document_id: get_text(&row, 1).parse().unwrap_or_default(),
                chunk_index: get_i64(&row, 2) as i32,
                content: get_text(&row, 3),
                embedding: None,
                created_at: get_ts(&row, 4),
            });
        }
        Ok(chunks)
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let pre_limit = config.pre_fusion_limit as i64;

        let fts_results = if config.use_fts {
            // Expand query with morphological variants for better FTS recall.
            let keywords = expand_query_keywords(query);
            // Sanitize query for FTS5: quote each word individually so special
            // characters (hyphens, colons, etc.) aren't interpreted as FTS5
            // operators. e.g. "time-sensitive notes" → `"time" "sensitive" "notes"`
            let sanitized_query: String = if keywords.is_empty() {
                query
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .filter(|w| !w.is_empty())
                    .map(|w| format!("\"{}\"", w))
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                keywords
                    .iter()
                    .map(|w| format!("\"{}\"", w))
                    .collect::<Vec<_>>()
                    .join(" OR ")
            };

            if sanitized_query.is_empty() {
                Vec::new()
            } else {
                let mut rows = conn
                    .query(
                        r#"
                    SELECT c.id, c.document_id, d.path, c.content, c.created_at
                    FROM memory_chunks_fts fts
                    JOIN memory_chunks c ON c._rowid = fts.rowid
                    JOIN memory_documents d ON d.id = c.document_id
                    WHERE d.user_id = ?1 AND d.agent_id IS ?2
                      AND memory_chunks_fts MATCH ?3
                    ORDER BY rank
                    LIMIT ?4
                    "#,
                        params![user_id, agent_id_str.as_deref(), sanitized_query, pre_limit],
                    )
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("FTS query failed: {}", e),
                    })?;

                let mut results = Vec::new();
                while let Some(row) =
                    rows.next()
                        .await
                        .map_err(|e| WorkspaceError::SearchFailed {
                            reason: format!("FTS row fetch failed: {}", e),
                        })?
                {
                    results.push(RankedResult {
                        chunk_id: get_text(&row, 0).parse().unwrap_or_default(),
                        document_id: get_text(&row, 1).parse().unwrap_or_default(),
                        path: get_text(&row, 2),
                        content: get_text(&row, 3),
                        rank: results.len() as u32 + 1,
                        created_at: get_opt_ts(&row, 4),
                        embedding: None,
                    });
                }
                results
            } // end: else (sanitized_query not empty)
        } else {
            Vec::new()
        };

        let vector_results = if let (true, Some(emb)) = (config.use_vector, embedding) {
            if emb.len() == LIBSQL_VECTOR_DIM {
                let vector_json = format!(
                    "[{}]",
                    emb.iter()
                        .map(|f| f.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );

                let mut rows = conn
                    .query(
                        r#"
                    SELECT c.id, c.document_id, d.path, c.content, c.created_at, c.embedding
                    FROM vector_top_k('idx_memory_chunks_embedding', vector(?1), ?2) AS top_k
                    JOIN memory_chunks c ON c._rowid = top_k.id
                    JOIN memory_documents d ON d.id = c.document_id
                    WHERE d.user_id = ?3 AND d.agent_id IS ?4
                    "#,
                        params![vector_json, pre_limit, user_id, agent_id_str.as_deref()],
                    )
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("Vector query failed: {}", e),
                    })?;

                let mut results = Vec::new();
                while let Some(row) =
                    rows.next()
                        .await
                        .map_err(|e| WorkspaceError::SearchFailed {
                            reason: format!("Vector row fetch failed: {}", e),
                        })?
                {
                    results.push(RankedResult {
                        chunk_id: get_text(&row, 0).parse().unwrap_or_default(),
                        document_id: get_text(&row, 1).parse().unwrap_or_default(),
                        path: get_text(&row, 2),
                        content: get_text(&row, 3),
                        rank: results.len() as u32 + 1,
                        created_at: get_opt_ts(&row, 4),
                        embedding: row
                            .get::<Vec<u8>>(5)
                            .ok()
                            .and_then(|bytes| deserialize_libsql_embedding(&bytes)),
                    });
                }
                results
            } else {
                let query_dim = emb.len() as i64;
                let mut rows = conn
                    .query(
                        r#"
                    SELECT c.id, c.document_id, d.path, c.content, c.created_at, c.embedding_blob
                    FROM memory_chunks c
                    JOIN memory_documents d ON d.id = c.document_id
                    WHERE d.user_id = ?1 AND d.agent_id IS ?2
                      AND c.embedding_blob IS NOT NULL
                      AND c.embedding_dim = ?3
                    LIMIT ?4
                    "#,
                        params![user_id, agent_id_str.as_deref(), query_dim, pre_limit],
                    )
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("Vector candidate query failed: {}", e),
                    })?;

                let mut scored = Vec::new();
                while let Some(row) =
                    rows.next()
                        .await
                        .map_err(|e| WorkspaceError::SearchFailed {
                            reason: format!("Vector row fetch failed: {}", e),
                        })?
                {
                    let candidate_embedding = row
                        .get::<Vec<u8>>(5)
                        .ok()
                        .and_then(|bytes| deserialize_libsql_embedding(&bytes));
                    if let Some(candidate_embedding) = candidate_embedding
                        && let Some(score) = cosine_similarity(emb, &candidate_embedding)
                    {
                        scored.push((
                            score,
                            RankedResult {
                                chunk_id: get_text(&row, 0).parse().unwrap_or_default(),
                                document_id: get_text(&row, 1).parse().unwrap_or_default(),
                                path: get_text(&row, 2),
                                content: get_text(&row, 3),
                                rank: 0,
                                created_at: get_opt_ts(&row, 4),
                                embedding: Some(candidate_embedding),
                            },
                        ));
                    }
                }
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                scored
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (_, mut result))| {
                        result.rank = (idx + 1) as u32;
                        result
                    })
                    .collect()
            }
        } else {
            Vec::new()
        };

        if embedding.is_some() && !config.use_vector {
            tracing::warn!(
                "Embedding provided but vector search is disabled in config; using FTS-only results"
            );
        }

        // Collect timestamps and embeddings from raw results before fusion.
        let mut doc_timestamps = HashMap::new();
        let mut chunk_embeddings = HashMap::new();
        for r in fts_results.iter().chain(vector_results.iter()) {
            if let Some(ts) = r.created_at {
                doc_timestamps.entry(r.document_id).or_insert(ts);
            }
            if let Some(ref embedding) = r.embedding {
                chunk_embeddings
                    .entry(r.chunk_id)
                    .or_insert_with(|| embedding.clone());
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
}
