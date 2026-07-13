-- Distinguish product surfaces that share the canonical conversation store.
-- Existing rows predate ThinClaw Desktop history unification and belong to
-- the agent runtime. Direct Workbench rows are imported explicitly as
-- `direct_workbench` by the Desktop migration.

ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS surface TEXT NOT NULL DEFAULT 'agent_cockpit';

UPDATE conversations
SET surface = 'agent_cockpit'
WHERE surface IS NULL OR surface = '';

CREATE INDEX IF NOT EXISTS idx_conversations_surface_activity
    ON conversations (surface, last_activity DESC);
