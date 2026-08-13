-- Bind every newly created sub-agent run to the authenticated gateway
-- principal and actor. Existing rows remain NULL so only the compatibility
-- primary bearer can see pre-ownership history.

ALTER TABLE subagent_runs ADD COLUMN principal_id TEXT;
ALTER TABLE subagent_runs ADD COLUMN actor_id TEXT;

CREATE INDEX idx_subagent_runs_owner_spawned
    ON subagent_runs (principal_id, actor_id, spawned_at DESC);
