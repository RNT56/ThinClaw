-- Durable inbox and observability for event-triggered routines.
--
-- Persists inbound event messages so matching can recover after restart, and
-- records per-routine evaluation decisions for debugging and UI inspection.

CREATE TABLE routine_event_inbox (
    id UUID PRIMARY KEY,
    principal_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    raw_sender_id TEXT NOT NULL,
    conversation_scope_id UUID NOT NULL,
    stable_external_conversation_key TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'pending',
    diagnostics JSONB NOT NULL DEFAULT '{}'::jsonb,
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    processed_at TIMESTAMPTZ,
    error_message TEXT,
    matched_routines INTEGER NOT NULL DEFAULT 0,
    fired_routines INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_routine_event_inbox_status_created
    ON routine_event_inbox (status, created_at ASC);
CREATE INDEX idx_routine_event_inbox_claimed_at
    ON routine_event_inbox (claimed_at)
    WHERE status = 'processing';
CREATE INDEX idx_routine_event_inbox_actor_created
    ON routine_event_inbox (principal_id, actor_id, created_at DESC);

CREATE TABLE routine_event_evaluations (
    id UUID PRIMARY KEY,
    event_id UUID NOT NULL REFERENCES routine_event_inbox(id) ON DELETE CASCADE,
    routine_id UUID NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    decision TEXT NOT NULL,
    reason TEXT,
    sequence_num INTEGER NOT NULL DEFAULT 0,
    channel TEXT NOT NULL,
    content_preview TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (event_id, routine_id)
);

CREATE INDEX idx_routine_event_evaluations_routine_created
    ON routine_event_evaluations (routine_id, created_at DESC);
CREATE INDEX idx_routine_event_evaluations_event
    ON routine_event_evaluations (event_id, sequence_num ASC);
