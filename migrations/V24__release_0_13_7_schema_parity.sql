-- Release 0.13.7 PostgreSQL schema parity.
--
-- Mirrors libSQL columns added during the release train so fresh Postgres
-- databases match the application storage layer and schema divergence checks.

ALTER TABLE agent_workspaces
    ADD COLUMN IF NOT EXISTS tool_profile TEXT;

ALTER TABLE experiment_runner_profiles
    ADD COLUMN IF NOT EXISTS readiness_class JSONB NOT NULL DEFAULT '"manual_only"'::jsonb;

ALTER TABLE experiment_runner_profiles
    ADD COLUMN IF NOT EXISTS launch_eligible BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS owner_user_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_experiment_campaigns_owner_created
    ON experiment_campaigns (owner_user_id, created_at DESC);
