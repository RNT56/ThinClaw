-- Durable routine delivery controls, cron trigger queueing, and direct-job recovery state.

ALTER TABLE routines
    ADD COLUMN IF NOT EXISTS policy_config JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE routines
    ADD COLUMN IF NOT EXISTS config_version BIGINT NOT NULL DEFAULT 1;

ALTER TABLE routine_runs
    ADD COLUMN IF NOT EXISTS trigger_key TEXT;

CREATE INDEX IF NOT EXISTS idx_routine_runs_routine_trigger_key
    ON routine_runs (routine_id, trigger_key)
    WHERE trigger_key IS NOT NULL;

ALTER TABLE routine_event_inbox
    ADD COLUMN IF NOT EXISTS event_type TEXT NOT NULL DEFAULT 'message';

ALTER TABLE routine_event_inbox
    ADD COLUMN IF NOT EXISTS idempotency_key TEXT NOT NULL DEFAULT '';

ALTER TABLE routine_event_inbox
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ;

ALTER TABLE routine_event_inbox
    ADD COLUMN IF NOT EXISTS attempt_count INTEGER NOT NULL DEFAULT 0;

UPDATE routine_event_inbox
SET idempotency_key = id::text
WHERE idempotency_key IS NULL OR idempotency_key = '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_routine_event_inbox_idempotency
    ON routine_event_inbox (idempotency_key);

ALTER TABLE routine_event_evaluations
    ADD COLUMN IF NOT EXISTS details JSONB NOT NULL DEFAULT '{}'::jsonb;

CREATE TABLE IF NOT EXISTS routine_trigger_queue (
    id UUID PRIMARY KEY,
    routine_id UUID NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    trigger_kind TEXT NOT NULL,
    trigger_label TEXT,
    due_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    decision TEXT,
    active_key TEXT,
    idempotency_key TEXT NOT NULL,
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    lease_expires_at TIMESTAMPTZ,
    processed_at TIMESTAMPTZ,
    error_message TEXT,
    diagnostics JSONB NOT NULL DEFAULT '{}'::jsonb,
    coalesced_count INTEGER NOT NULL DEFAULT 0,
    backlog_collapsed BOOLEAN NOT NULL DEFAULT FALSE,
    routine_config_version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_routine_trigger_queue_active_key
    ON routine_trigger_queue (active_key)
    WHERE active_key IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_routine_trigger_queue_idempotency
    ON routine_trigger_queue (idempotency_key);

CREATE INDEX IF NOT EXISTS idx_routine_trigger_queue_status_due
    ON routine_trigger_queue (status, due_at ASC);

CREATE INDEX IF NOT EXISTS idx_routine_trigger_queue_routine_created
    ON routine_trigger_queue (routine_id, created_at DESC);
