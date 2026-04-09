-- Actor-scoped history, routines, and jobs.
--
-- Extends the identity registry with actor-aware conversation attribution,
-- private routine/job ownership, and conversation handoff metadata support.

ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS actor_id TEXT;

ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS conversation_scope_id UUID;

ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS conversation_kind TEXT NOT NULL DEFAULT 'direct';

ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS stable_external_conversation_key TEXT;

UPDATE conversations
SET
    actor_id = COALESCE(actor_id, user_id),
    conversation_scope_id = COALESCE(conversation_scope_id, id),
    stable_external_conversation_key = COALESCE(
        stable_external_conversation_key,
        channel || ':' || COALESCE(thread_id, id::text)
    )
WHERE actor_id IS NULL
   OR conversation_scope_id IS NULL
   OR stable_external_conversation_key IS NULL;

CREATE INDEX IF NOT EXISTS idx_conversations_actor ON conversations (actor_id);
CREATE INDEX IF NOT EXISTS idx_conversations_scope ON conversations (conversation_scope_id);
CREATE INDEX IF NOT EXISTS idx_conversations_kind ON conversations (conversation_kind);
CREATE INDEX IF NOT EXISTS idx_conversations_actor_kind_activity
    ON conversations (user_id, actor_id, conversation_kind, last_activity DESC);

ALTER TABLE conversation_messages
    ADD COLUMN IF NOT EXISTS actor_id TEXT;

ALTER TABLE conversation_messages
    ADD COLUMN IF NOT EXISTS actor_display_name TEXT;

ALTER TABLE conversation_messages
    ADD COLUMN IF NOT EXISTS raw_sender_id TEXT;

ALTER TABLE conversation_messages
    ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}'::jsonb;

CREATE INDEX IF NOT EXISTS idx_conversation_messages_actor
    ON conversation_messages (actor_id);

ALTER TABLE routines
    ADD COLUMN IF NOT EXISTS actor_id TEXT;

UPDATE routines
SET actor_id = COALESCE(actor_id, user_id, 'default')
WHERE actor_id IS NULL;

ALTER TABLE routines
    ALTER COLUMN actor_id SET DEFAULT 'default';

ALTER TABLE routines
    ALTER COLUMN actor_id SET NOT NULL;

ALTER TABLE routines
    DROP CONSTRAINT IF EXISTS routines_user_id_name_key;

CREATE UNIQUE INDEX IF NOT EXISTS idx_routines_user_actor_name
    ON routines (user_id, actor_id, name);

CREATE INDEX IF NOT EXISTS idx_routines_actor
    ON routines (actor_id);

ALTER TABLE agent_jobs
    ADD COLUMN IF NOT EXISTS actor_id TEXT;

UPDATE agent_jobs
SET actor_id = COALESCE(actor_id, user_id, 'default')
WHERE actor_id IS NULL;

ALTER TABLE agent_jobs
    ALTER COLUMN actor_id SET DEFAULT 'default';

ALTER TABLE agent_jobs
    ALTER COLUMN actor_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_agent_jobs_actor
    ON agent_jobs (actor_id);
