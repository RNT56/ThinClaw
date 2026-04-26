-- Outcome-backed learning contracts and observations.
--
-- Adds deferred consequence tracking on top of the existing learning ledger.

CREATE TABLE IF NOT EXISTS outcome_contracts (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    actor_id TEXT,
    channel TEXT,
    thread_id TEXT,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    contract_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    summary TEXT,
    due_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    final_verdict TEXT,
    final_score DOUBLE PRECISION,
    evaluation_details JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    dedupe_key TEXT NOT NULL,
    claimed_at TIMESTAMPTZ,
    evaluated_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_outcome_contracts_dedupe_key
    ON outcome_contracts (dedupe_key);
CREATE INDEX IF NOT EXISTS idx_outcome_contracts_status_due
    ON outcome_contracts (status, due_at);
CREATE INDEX IF NOT EXISTS idx_outcome_contracts_user_actor_thread_status
    ON outcome_contracts (user_id, actor_id, thread_id, status);
CREATE INDEX IF NOT EXISTS idx_outcome_contracts_source
    ON outcome_contracts (source_kind, source_id);

CREATE TABLE IF NOT EXISTS outcome_observations (
    id UUID PRIMARY KEY,
    contract_id UUID NOT NULL REFERENCES outcome_contracts(id) ON DELETE CASCADE,
    observation_kind TEXT NOT NULL,
    polarity TEXT NOT NULL,
    weight DOUBLE PRECISION NOT NULL DEFAULT 0,
    summary TEXT,
    evidence JSONB NOT NULL DEFAULT '{}'::jsonb,
    fingerprint TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_outcome_observations_contract_fingerprint
    ON outcome_observations (contract_id, fingerprint);
CREATE INDEX IF NOT EXISTS idx_outcome_observations_contract_observed_at
    ON outcome_observations (contract_id, observed_at DESC);
