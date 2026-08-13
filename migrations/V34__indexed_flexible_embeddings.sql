-- Record the embedding model that produced each vector. Dimension equality is
-- necessary for a pgvector comparison, but it is not sufficient: two models
-- with the same output width do not share a semantic vector space.
--
-- Existing embeddings deliberately remain NULL/unknown. The application
-- treats them as stale and re-embeds them with the active model; it never
-- guesses a legacy model identity.
ALTER TABLE memory_chunks
    ADD COLUMN embedding_model TEXT;

ALTER TABLE memory_chunks
    ADD CONSTRAINT memory_chunks_embedding_model_shape
    CHECK (
        embedding_model IS NULL OR (
            octet_length(embedding_model) BETWEEN 1 AND 512
            AND embedding_model !~ '[[:cntrl:]]'
        )
    ) NOT VALID;

-- Validation uses a low-conflict lock and does not rewrite the vector column.
-- Dimension-specific HNSW indexes are intentionally not created here:
-- refinery executes each migration in a transaction, while safe online
-- backfill requires CREATE INDEX CONCURRENTLY. Store::run_migrations performs
-- that idempotent, advisory-lock-protected reconciliation after commit.
ALTER TABLE memory_chunks
    VALIDATE CONSTRAINT memory_chunks_embedding_model_shape;
