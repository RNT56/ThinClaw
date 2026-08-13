# PostgreSQL vector search

ThinClaw stores PostgreSQL workspace embeddings in an unbounded-typmod
`vector` column. This permits an embedding provider to change dimensions
without rewriting the table. Search remains safe and indexed by treating the
embedding model name and dimension as one immutable profile.

## Supported profiles

The application creates partial HNSW expression indexes for these built-in
dimensions:

| Dimensions | Indexed expression | Operator class |
| ---: | --- | --- |
| 256, 384, 512, 768, 1024, 1536 | `embedding::vector(n)` | `vector_cosine_ops` |
| 3072 | `embedding::halfvec(3072)` | `halfvec_cosine_ops` |

pgvector limits full-precision `vector` ANN indexes to 2,000 dimensions, so
the 3,072-dimension profile uses a half-precision expression index. The
canonical stored vector remains full precision. pgvector 0.7.0 or newer is
therefore required. Startup fails with an actionable error when `halfvec` is
not available; upgrade the extension before upgrading ThinClaw:

```sql
ALTER EXTENSION vector UPDATE;
```

Other pgvector dimensions from 1 through 16,000 remain supported for small
workspaces through exact cosine search. Exact search emits a warning and is
limited to 10,000 candidates after tenant, agent, model, dimension, and path
filtering. Above that limit the query fails closed and asks the operator to
select an indexed dimension and re-embed.

libSQL retains its existing 1,536-dimension native vector index. Other libSQL
dimensions use the same bounded exact-search policy over canonical little-
endian `f32` payloads.

## Upgrade and backfill

Migration V34 adds `memory_chunks.embedding_model`. Existing rows deliberately
keep a NULL model because a dimension alone cannot identify the semantic vector
space that produced an embedding. These legacy rows are excluded from vector
search and selected by the profile-aware backfill query. The next workspace
backfill re-embeds rows whose vector is missing or whose model or dimension does
not match the active provider.

The schema migration does not build HNSW indexes because refinery runs each
migration in a transaction and PostgreSQL forbids `CREATE INDEX CONCURRENTLY`
there. Immediately after migrations commit, startup performs an idempotent
index reconciliation:

1. Acquire a session advisory lock so only one ThinClaw instance reconciles.
2. Verify each expected index is both ready and valid.
3. Drop a crash-left invalid index with `DROP INDEX CONCURRENTLY`.
4. Build each missing index with `CREATE INDEX CONCURRENTLY`.

Existing reads and writes remain available while indexes build. Allow startup
to finish before declaring the upgraded instance healthy. The regular
PostgreSQL integration suite deletes one managed index and proves that the next
migration run repairs it.

## Query safety

Every vector write and query must carry a non-empty model identity and a
non-zero, finite vector. PostgreSQL queries include a literal `vector_dims`
predicate matching their expression index and an exact model predicate. libSQL
applies the same profile filters. This prevents:

- pgvector dimension mismatch errors in a mixed-dimension table;
- comparison across unrelated models that happen to emit the same dimension;
- stale legacy embeddings entering recall;
- non-finite or zero-norm values reaching cosine distance operators.

If query embedding generation fails or produces an invalid shape, hybrid
search can continue with full-text recall. Vector-only search returns an error.

## Benchmark and plan verification

PR CI runs the PostgreSQL integration contract that asserts every supported
dimension produces a valid plan using its managed HNSW index. Nightly CI also
runs a deterministic benchmark at 1,000 and 10,000 rows for 384, 1,536, and
3,072 dimensions. It warms both paths, records JSON `EXPLAIN (ANALYZE, BUFFERS)`
plans, takes the best of three repetitions, and requires at least a 1.25x ANN
speedup at the largest row count.

Run the same benchmark locally against an isolated pgvector database:

```bash
export DATABASE_URL=postgres://thinclaw:thinclaw@localhost:5432/thinclaw_test
export THINCLAW_REQUIRE_POSTGRES_INTEGRATION=1
export THINCLAW_VECTOR_BENCH_DIMENSIONS=384,1536,3072
export THINCLAW_VECTOR_BENCH_ROW_COUNTS=1000,10000
export THINCLAW_VECTOR_BENCH_MIN_SPEEDUP=1.25
cargo test --locked --test workspace_integration \
  --no-default-features --features postgres \
  benchmark_postgres_vector_ann_against_exact_scan \
  -- --ignored --nocapture --test-threads=1
```

The test writes only users prefixed `vector_benchmark_` and removes them after
each dimension. Use a dedicated database so interruption cannot leave benchmark
rows in an operational workspace.

## Rollback

The new indexes are additive. To roll the application back while leaving the
V34 schema in place, remove them online if the older release should not retain
their storage cost:

```sql
DROP INDEX CONCURRENTLY IF EXISTS idx_memory_chunks_embedding_hnsw_256_v1;
DROP INDEX CONCURRENTLY IF EXISTS idx_memory_chunks_embedding_hnsw_384_v1;
DROP INDEX CONCURRENTLY IF EXISTS idx_memory_chunks_embedding_hnsw_512_v1;
DROP INDEX CONCURRENTLY IF EXISTS idx_memory_chunks_embedding_hnsw_768_v1;
DROP INDEX CONCURRENTLY IF EXISTS idx_memory_chunks_embedding_hnsw_1024_v1;
DROP INDEX CONCURRENTLY IF EXISTS idx_memory_chunks_embedding_hnsw_1536_v1;
DROP INDEX CONCURRENTLY IF EXISTS idx_memory_chunks_embedding_hnsw_3072_half_v1;
```

Do not drop `embedding_model` during a live rollback: doing so rewrites the
contract and lets old code mix semantic profiles. A full schema downgrade is
safe only after stopping all writers, exporting a backup, choosing one model
and dimension, re-embedding every retained row to that profile, and restoring
the older fixed-dimension index.
