//! Shared pgvector policy for both PostgreSQL workspace repository surfaces.
//!
//! `memory_chunks.embedding` intentionally remains an unbounded-typmod
//! `vector` column so an installation can change embedding models without a
//! destructive table rewrite. ANN indexes, however, require a fixed dimension.
//! We therefore reconcile one partial expression HNSW index per supported
//! dimension after transactional schema migrations have committed.

use deadpool_postgres::Pool;
use pgvector::Vector;
use thinclaw_types::error::WorkspaceError;
use tokio_postgres::GenericClient;
use uuid::Uuid;

use crate::search::RankedResult;

/// Maximum dimension accepted by pgvector's `vector` storage type.
pub const MAX_POSTGRES_VECTOR_DIM: usize = 16_000;

/// Exact search is deliberately bounded. Larger unindexed profiles must use a
/// supported ANN dimension instead of turning every query into an unbounded
/// sequential distance scan.
pub const MAX_EXACT_VECTOR_CANDIDATES: i64 = 10_000;

/// Largest finite value representable by pgvector's halfvec storage.
const MAX_HALF_VECTOR_VALUE: f32 = 65_504.0;

/// A partial HNSW index specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnIndexSpec {
    pub dimension: usize,
    pub name: &'static str,
    pub create_sql: &'static str,
    distance_sql: &'static str,
}

/// Dimensions emitted by ThinClaw's built-in embedding configurations.
/// Dimensions through 1536 retain full precision; 3072 uses pgvector's
/// half-precision expression index because fp32 HNSW is capped at 2000.
pub const ANN_INDEXES: &[AnnIndexSpec] = &[
    AnnIndexSpec {
        dimension: 256,
        name: "idx_memory_chunks_embedding_hnsw_256_v1",
        create_sql: "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_chunks_embedding_hnsw_256_v1 ON memory_chunks USING hnsw ((embedding::vector(256)) vector_cosine_ops) WITH (m = 16, ef_construction = 64) WHERE vector_dims(embedding) = 256",
        distance_sql: "(c.embedding::vector(256)) <=> $3",
    },
    AnnIndexSpec {
        dimension: 384,
        name: "idx_memory_chunks_embedding_hnsw_384_v1",
        create_sql: "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_chunks_embedding_hnsw_384_v1 ON memory_chunks USING hnsw ((embedding::vector(384)) vector_cosine_ops) WITH (m = 16, ef_construction = 64) WHERE vector_dims(embedding) = 384",
        distance_sql: "(c.embedding::vector(384)) <=> $3",
    },
    AnnIndexSpec {
        dimension: 512,
        name: "idx_memory_chunks_embedding_hnsw_512_v1",
        create_sql: "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_chunks_embedding_hnsw_512_v1 ON memory_chunks USING hnsw ((embedding::vector(512)) vector_cosine_ops) WITH (m = 16, ef_construction = 64) WHERE vector_dims(embedding) = 512",
        distance_sql: "(c.embedding::vector(512)) <=> $3",
    },
    AnnIndexSpec {
        dimension: 768,
        name: "idx_memory_chunks_embedding_hnsw_768_v1",
        create_sql: "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_chunks_embedding_hnsw_768_v1 ON memory_chunks USING hnsw ((embedding::vector(768)) vector_cosine_ops) WITH (m = 16, ef_construction = 64) WHERE vector_dims(embedding) = 768",
        distance_sql: "(c.embedding::vector(768)) <=> $3",
    },
    AnnIndexSpec {
        dimension: 1024,
        name: "idx_memory_chunks_embedding_hnsw_1024_v1",
        create_sql: "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_chunks_embedding_hnsw_1024_v1 ON memory_chunks USING hnsw ((embedding::vector(1024)) vector_cosine_ops) WITH (m = 16, ef_construction = 64) WHERE vector_dims(embedding) = 1024",
        distance_sql: "(c.embedding::vector(1024)) <=> $3",
    },
    AnnIndexSpec {
        dimension: 1536,
        name: "idx_memory_chunks_embedding_hnsw_1536_v1",
        create_sql: "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_chunks_embedding_hnsw_1536_v1 ON memory_chunks USING hnsw ((embedding::vector(1536)) vector_cosine_ops) WITH (m = 16, ef_construction = 64) WHERE vector_dims(embedding) = 1536",
        distance_sql: "(c.embedding::vector(1536)) <=> $3",
    },
    AnnIndexSpec {
        dimension: 3072,
        name: "idx_memory_chunks_embedding_hnsw_3072_half_v1",
        create_sql: "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_chunks_embedding_hnsw_3072_half_v1 ON memory_chunks USING hnsw ((embedding::halfvec(3072)) halfvec_cosine_ops) WITH (m = 16, ef_construction = 64) WHERE vector_dims(embedding) = 3072",
        distance_sql: "(c.embedding::halfvec(3072)) <=> (($3::vector)::halfvec(3072))",
    },
];

pub fn ann_index_for_dimension(dimension: usize) -> Option<&'static AnnIndexSpec> {
    ANN_INDEXES
        .iter()
        .find(|index| index.dimension == dimension)
}

pub fn validate_embedding_dimension(dimension: usize) -> Result<(), WorkspaceError> {
    if dimension == 0 || dimension > MAX_POSTGRES_VECTOR_DIM {
        return Err(WorkspaceError::EmbeddingFailed {
            reason: format!(
                "embedding dimension {} is outside PostgreSQL's supported range 1..={MAX_POSTGRES_VECTOR_DIM}",
                dimension
            ),
        });
    }
    Ok(())
}

pub fn validate_embedding(embedding: &[f32]) -> Result<(), WorkspaceError> {
    validate_embedding_dimension(embedding.len())?;
    if embedding.iter().any(|value| !value.is_finite()) {
        return Err(WorkspaceError::EmbeddingFailed {
            reason: "embedding contains a non-finite value".to_string(),
        });
    }
    if embedding.iter().all(|value| *value == 0.0) {
        return Err(WorkspaceError::EmbeddingFailed {
            reason: "zero-norm embeddings cannot participate in cosine search".to_string(),
        });
    }
    if embedding.len() == 3072
        && embedding
            .iter()
            .any(|value| value.abs() > MAX_HALF_VECTOR_VALUE)
    {
        return Err(WorkspaceError::EmbeddingFailed {
            reason: "3072-dimension embeddings must fit pgvector halfvec for indexed search"
                .to_string(),
        });
    }
    Ok(())
}

pub fn validate_embedding_model(model: Option<&str>) -> Result<(), WorkspaceError> {
    let Some(model) = model else {
        return Err(WorkspaceError::EmbeddingFailed {
            reason: "embedding model identity is required for vector storage and search"
                .to_string(),
        });
    };
    if model.is_empty() || model.len() > 512 || model.chars().any(char::is_control) {
        return Err(WorkspaceError::EmbeddingFailed {
            reason: "embedding model identity is empty, oversized, or malformed".to_string(),
        });
    }
    Ok(())
}

pub fn validate_embedding_with_model(
    embedding: &[f32],
    model: Option<&str>,
) -> Result<(), WorkspaceError> {
    validate_embedding(embedding)?;
    validate_embedding_model(model)
}

/// Reconcile ANN indexes outside refinery's per-migration transaction.
///
/// `CREATE INDEX CONCURRENTLY` keeps existing workspaces writable during the
/// backfill. A session advisory lock serializes concurrent app instances, and
/// invalid indexes left by a crash are dropped and rebuilt on the next start.
pub async fn ensure_postgres_vector_indexes(pool: &Pool) -> Result<(), String> {
    let client = pool
        .get()
        .await
        .map_err(|error| format!("get PostgreSQL connection for vector indexes: {error}"))?;
    let halfvec_available: bool = client
        .query_one("SELECT to_regtype('halfvec') IS NOT NULL", &[])
        .await
        .map_err(|error| format!("inspect pgvector halfvec support: {error}"))?
        .get(0);
    if !halfvec_available {
        return Err(
            "pgvector 0.7.0 or newer is required for the indexed 3072-dimension embedding profile; run ALTER EXTENSION vector UPDATE before upgrading ThinClaw"
                .to_string(),
        );
    }

    client
        .query_one("SELECT pg_advisory_lock(8423702026)", &[])
        .await
        .map_err(|error| format!("lock PostgreSQL vector-index reconciliation: {error}"))?;
    let result = reconcile_indexes(&client).await;
    let unlock = client
        .query_one("SELECT pg_advisory_unlock(8423702026)", &[])
        .await
        .map_err(|error| format!("unlock PostgreSQL vector-index reconciliation: {error}"));
    result.and(unlock.map(|_| ()))
}

async fn reconcile_indexes(client: &tokio_postgres::Client) -> Result<(), String> {
    for index in ANN_INDEXES {
        let state = client
            .query_opt(
                r#"
                SELECT pg_index.indisvalid AND pg_index.indisready
                FROM pg_class AS index_relation
                JOIN pg_namespace AS namespace
                  ON namespace.oid = index_relation.relnamespace
                JOIN pg_index ON pg_index.indexrelid = index_relation.oid
                WHERE namespace.nspname = current_schema()
                  AND index_relation.relname = $1
                "#,
                &[&index.name],
            )
            .await
            .map_err(|error| format!("inspect vector index {}: {error}", index.name))?
            .map(|row| row.get::<_, bool>(0));

        if state == Some(false) {
            let drop_sql = format!("DROP INDEX CONCURRENTLY IF EXISTS {}", index.name);
            client
                .batch_execute(&drop_sql)
                .await
                .map_err(|error| format!("drop invalid vector index {}: {error}", index.name))?;
        }
        if state != Some(true) {
            client
                .batch_execute(index.create_sql)
                .await
                .map_err(|error| format!("build vector index {}: {error}", index.name))?;
        }
    }
    Ok(())
}

fn postgres_vector_search_sql_with_distance(
    dimension: usize,
    include_embeddings: bool,
    distance: &str,
) -> String {
    let embedding_projection = if include_embeddings {
        ", c.embedding"
    } else {
        ""
    };
    format!(
        r#"
        SELECT c.id AS chunk_id, c.document_id, d.path, c.content,
               1 - ({distance}) AS similarity,
               d.updated_at AS created_at{embedding_projection}
        FROM memory_chunks c
        JOIN memory_documents d ON d.id = c.document_id
        WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
          AND c.embedding IS NOT NULL
          AND vector_dims(c.embedding) = {dimension}
          AND ($6::text IS NULL OR c.embedding_model = $6)
          AND (
              cardinality($5::text[]) = 0 OR EXISTS (
                  SELECT 1 FROM unnest($5::text[]) AS allowed(prefix)
                  WHERE d.path = allowed.prefix
                     OR left(d.path, length(allowed.prefix) + 1) = allowed.prefix || '/'
              )
          )
        ORDER BY {distance}
        LIMIT $4
        "#
    )
}

#[doc(hidden)]
pub fn postgres_vector_search_sql(dimension: usize, include_embeddings: bool) -> String {
    let distance = ann_index_for_dimension(dimension)
        .map(|index| index.distance_sql)
        .unwrap_or("c.embedding <=> $3");
    postgres_vector_search_sql_with_distance(dimension, include_embeddings, distance)
}

/// Exact equivalent of the production query, exposed for the reproducible
/// ANN benchmark. The raw unbounded-typmod expression deliberately cannot use
/// a dimension-specific expression index.
#[doc(hidden)]
pub fn postgres_exact_vector_search_sql(dimension: usize, include_embeddings: bool) -> String {
    postgres_vector_search_sql_with_distance(dimension, include_embeddings, "c.embedding <=> $3")
}

async fn exact_candidate_count(
    client: &(impl GenericClient + Sync),
    user_id: &str,
    agent_id: Option<Uuid>,
    dimension: i32,
    embedding_model: Option<&str>,
    path_prefixes: &[String],
) -> Result<i64, WorkspaceError> {
    client
        .query_one(
            r#"
            SELECT COUNT(*)
            FROM memory_chunks c
            JOIN memory_documents d ON d.id = c.document_id
            WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
              AND c.embedding IS NOT NULL
              AND vector_dims(c.embedding) = $3
              AND ($4::text IS NULL OR c.embedding_model = $4)
              AND (
                  cardinality($5::text[]) = 0 OR EXISTS (
                      SELECT 1 FROM unnest($5::text[]) AS allowed(prefix)
                      WHERE d.path = allowed.prefix
                         OR left(d.path, length(allowed.prefix) + 1) = allowed.prefix || '/'
                  )
              )
            "#,
            &[
                &user_id,
                &agent_id,
                &dimension,
                &embedding_model,
                &path_prefixes,
            ],
        )
        .await
        .map(|row| row.get::<_, i64>(0))
        .map_err(|error| WorkspaceError::SearchFailed {
            reason: format!("exact vector candidate count failed: {error}"),
        })
}

/// Execute dimension- and model-safe PostgreSQL vector search.
pub async fn search_postgres_vectors(
    client: &(impl GenericClient + Sync),
    user_id: &str,
    agent_id: Option<Uuid>,
    embedding: &[f32],
    embedding_model: Option<&str>,
    limit: usize,
    include_embeddings: bool,
    path_prefixes: &[String],
) -> Result<Vec<RankedResult>, WorkspaceError> {
    validate_embedding_with_model(embedding, embedding_model)?;
    let dimension = embedding.len();
    if ann_index_for_dimension(dimension).is_none() {
        let dimension_i32 = i32::try_from(dimension).map_err(|_| WorkspaceError::SearchFailed {
            reason: "embedding dimension does not fit PostgreSQL integer metadata".to_string(),
        })?;
        let candidates = exact_candidate_count(
            client,
            user_id,
            agent_id,
            dimension_i32,
            embedding_model,
            path_prefixes,
        )
        .await?;
        if candidates > MAX_EXACT_VECTOR_CANDIDATES {
            return Err(WorkspaceError::SearchFailed {
                reason: format!(
                    "embedding dimension {dimension} has {candidates} exact-search candidates, exceeding the safe limit of {MAX_EXACT_VECTOR_CANDIDATES}; use an indexed dimension ({}) and re-embed the workspace",
                    ANN_INDEXES
                        .iter()
                        .map(|index| index.dimension.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
        tracing::warn!(
            dimension,
            candidates,
            threshold = MAX_EXACT_VECTOR_CANDIDATES,
            "Using bounded exact PostgreSQL vector search for an unindexed embedding dimension"
        );
    }

    let vector = Vector::from(embedding.to_vec());
    let sql = postgres_vector_search_sql(dimension, include_embeddings);
    let rows = client
        .query(
            &sql,
            &[
                &user_id,
                &agent_id,
                &vector,
                &(limit as i64),
                &path_prefixes,
                &embedding_model,
            ],
        )
        .await
        .map_err(|error| WorkspaceError::SearchFailed {
            reason: format!("dimension-safe vector query failed: {error}"),
        })?;

    Ok(rows
        .iter()
        .enumerate()
        .map(|(index, row)| RankedResult {
            chunk_id: row.get("chunk_id"),
            document_id: row.get("document_id"),
            path: row.get("path"),
            content: row.get("content"),
            rank: (index + 1) as u32,
            created_at: row.get("created_at"),
            embedding: include_embeddings
                .then(|| row.try_get::<_, Vector>("embedding").ok())
                .flatten()
                .map(|vector| vector.to_vec()),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_dimensions_have_unique_ann_indexes() {
        let dimensions = ANN_INDEXES
            .iter()
            .map(|index| index.dimension)
            .collect::<std::collections::BTreeSet<_>>();
        let names = ANN_INDEXES
            .iter()
            .map(|index| index.name)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(dimensions.len(), ANN_INDEXES.len());
        assert_eq!(names.len(), ANN_INDEXES.len());
        assert!(
            ann_index_for_dimension(3072)
                .unwrap()
                .create_sql
                .contains("halfvec")
        );
    }

    #[test]
    fn generated_queries_pin_dimension_and_model_filter() {
        let indexed = postgres_vector_search_sql(768, false);
        assert!(indexed.contains("embedding::vector(768)"));
        assert!(indexed.contains("vector_dims(c.embedding) = 768"));
        assert!(indexed.contains("c.embedding_model = $6"));

        let fallback = postgres_vector_search_sql(4096, true);
        assert!(fallback.contains("c.embedding <=> $3"));
        assert!(fallback.contains("vector_dims(c.embedding) = 4096"));
        assert!(fallback.contains(", c.embedding"));

        let exact = postgres_exact_vector_search_sql(768, false);
        assert!(exact.contains("c.embedding <=> $3"));
        assert!(!exact.contains("embedding::vector(768)"));
    }

    #[test]
    fn invalid_vectors_and_model_ids_fail_before_database_io() {
        assert!(validate_embedding_dimension(0).is_err());
        assert!(validate_embedding(&[]).is_err());
        assert!(validate_embedding(&[0.0, 0.0]).is_err());
        assert!(validate_embedding(&[f32::NAN]).is_err());
        assert!(validate_embedding(&vec![f32::MAX; 3072]).is_err());
        assert!(validate_embedding_model(None).is_err());
        assert!(validate_embedding_model(Some("")).is_err());
        assert!(validate_embedding_with_model(&[1.0, 0.0], Some("model-a")).is_ok());
    }
}
