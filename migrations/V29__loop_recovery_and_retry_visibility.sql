-- Durable claims, visibility delays, and quarantine state make loop recovery
-- restart-safe without immediately reclaiming deferred or terminal work.

ALTER TABLE routine_event_inbox
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_routine_event_inbox_status_next_attempt
    ON routine_event_inbox (status, next_attempt_at, created_at ASC);

ALTER TABLE routine_trigger_queue
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_routine_trigger_queue_status_next_attempt
    ON routine_trigger_queue (status, next_attempt_at, due_at ASC);

ALTER TABLE outcome_contracts
    ADD COLUMN IF NOT EXISTS claimed_by TEXT;

ALTER TABLE outcome_contracts
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ;

ALTER TABLE outcome_contracts
    ADD COLUMN IF NOT EXISTS attempt_count INTEGER NOT NULL DEFAULT 0;

ALTER TABLE outcome_contracts
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_outcome_contracts_status_retry
    ON outcome_contracts (status, next_attempt_at, lease_expires_at, due_at);

ALTER TABLE tool_failures
    ADD COLUMN IF NOT EXISTS quarantined_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_tool_failures_active_incident
    ON tool_failures (tool_name)
    WHERE repaired_at IS NULL AND quarantined_at IS NULL;
