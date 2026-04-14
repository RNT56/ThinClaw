-- Research platform persistence additions and migration parity.
--
-- Extends experiment artifacts, model usage, campaign, and trial records for
-- autonomous research flow metadata and policy attribution.

CREATE TABLE IF NOT EXISTS experiment_targets (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    kind JSONB NOT NULL DEFAULT '"prompt_asset"'::jsonb,
    location TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_experiment_targets_updated
    ON experiment_targets (updated_at DESC, name ASC);

CREATE TABLE IF NOT EXISTS experiment_target_links (
    id UUID PRIMARY KEY,
    target_id UUID NOT NULL REFERENCES experiment_targets(id) ON DELETE CASCADE,
    kind JSONB NOT NULL DEFAULT '"prompt_asset"'::jsonb,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    route_key TEXT NOT NULL DEFAULT '',
    logical_role TEXT NOT NULL DEFAULT '',
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (target_id, kind, provider, model, route_key, logical_role)
);

CREATE INDEX IF NOT EXISTS idx_experiment_target_links_lookup
    ON experiment_target_links (provider, model, updated_at DESC);

CREATE TABLE IF NOT EXISTS experiment_model_usage_records (
    id UUID PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    route_key TEXT,
    logical_role TEXT,
    endpoint_type TEXT,
    workload_tag TEXT,
    latency_ms BIGINT,
    cost_usd NUMERIC,
    success BOOLEAN NOT NULL DEFAULT TRUE,
    prompt_asset_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    retrieval_asset_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    tool_policy_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    evaluator_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    parser_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_experiment_model_usage_created
    ON experiment_model_usage_records (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_experiment_model_usage_provider_model
    ON experiment_model_usage_records (provider, model, created_at DESC);

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS queue_state TEXT NOT NULL DEFAULT 'not_queued';

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS queue_position INTEGER NOT NULL DEFAULT 0;

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS active_trial_id UUID;

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS total_runtime_ms BIGINT NOT NULL DEFAULT 0;

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS total_cost_usd NUMERIC(18, 8) NOT NULL DEFAULT 0;

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS consecutive_non_improving_trials INTEGER NOT NULL DEFAULT 0;

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS max_trials_override INTEGER;

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS gateway_url TEXT;

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS runtime_ms BIGINT;

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS attributed_cost_usd NUMERIC(18, 8);

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS hypothesis TEXT;

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS mutation_summary TEXT;

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS reviewer_decision TEXT;

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS provider_job_id TEXT;

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS provider_job_metadata JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE experiment_projects
    ADD COLUMN IF NOT EXISTS autonomy_mode JSONB NOT NULL DEFAULT '"autonomous"'::jsonb;
