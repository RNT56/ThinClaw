-- Durable repo-project run records and GitHub webhook delivery audit/idempotency.

CREATE TABLE IF NOT EXISTS repo_project_runs (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES repo_projects(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_repo_project_runs_project
    ON repo_project_runs(project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS repo_webhook_deliveries (
    delivery_id TEXT PRIMARY KEY,
    event TEXT NOT NULL,
    data JSONB NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_repo_webhook_deliveries_received
    ON repo_webhook_deliveries(received_at DESC);
