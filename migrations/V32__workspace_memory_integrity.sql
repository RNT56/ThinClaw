-- Ensure the shared (agent_id IS NULL) workspace namespace is genuinely unique.
-- SQL UNIQUE constraints treat NULL values as distinct, so the original
-- (user_id, agent_id, path) constraint allowed duplicate shared documents
-- during concurrent creation.

WITH duplicate_groups AS (
    SELECT
        user_id,
        path,
        (array_agg(id ORDER BY created_at ASC, id ASC))[1] AS keep_id,
        string_agg(DISTINCT NULLIF(content, ''), E'\n\n') AS merged_content,
        MAX(updated_at) AS newest_update
    FROM memory_documents
    WHERE agent_id IS NULL
    GROUP BY user_id, path
    HAVING COUNT(*) > 1
)
UPDATE memory_documents AS document
SET content = COALESCE(grouped.merged_content, ''),
    updated_at = grouped.newest_update,
    metadata = document.metadata || '{"index_dirty":true}'::jsonb
FROM duplicate_groups AS grouped
WHERE document.id = grouped.keep_id;

-- Merged content must be re-chunked. Remove every stale chunk in affected
-- groups; the durable dirty marker is repaired by Workspace before search.
WITH duplicate_keys AS (
    SELECT user_id, path
    FROM memory_documents
    WHERE agent_id IS NULL
    GROUP BY user_id, path
    HAVING COUNT(*) > 1
)
DELETE FROM memory_chunks AS chunk
USING memory_documents AS document, duplicate_keys AS duplicate
WHERE chunk.document_id = document.id
  AND document.agent_id IS NULL
  AND document.user_id = duplicate.user_id
  AND document.path = duplicate.path;

WITH ranked AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY user_id, path
            ORDER BY created_at ASC, id ASC
        ) AS duplicate_rank
    FROM memory_documents
    WHERE agent_id IS NULL
)
DELETE FROM memory_documents AS document
USING ranked
WHERE document.id = ranked.id
  AND ranked.duplicate_rank > 1;

CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_documents_shared_path_unique
    ON memory_documents (user_id, path)
    WHERE agent_id IS NULL;

-- Older writes predate durable index-state tracking. One bounded repair pass
-- per search progressively verifies/rebuilds them without blocking migration.
UPDATE memory_documents
SET metadata = metadata || '{"index_dirty":true}'::jsonb
WHERE NOT (metadata ? 'index_dirty');

-- Native channel threads are rehydrated by identity + external address after
-- restart. Keep both direct and group lookup paths indexed without creating a
-- uniqueness constraint: `/new` intentionally permits several internal
-- threads behind one external room, with the latest one winning.
CREATE INDEX IF NOT EXISTS idx_conversations_ingress_direct
    ON conversations (user_id, actor_id, channel, thread_id, last_activity DESC)
    WHERE conversation_kind = 'direct';

CREATE INDEX IF NOT EXISTS idx_conversations_ingress_group
    ON conversations (user_id, conversation_scope_id, last_activity DESC)
    WHERE conversation_kind = 'group';
