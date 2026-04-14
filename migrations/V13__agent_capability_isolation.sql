CREATE TABLE IF NOT EXISTS agent_workspaces (
    id UUID PRIMARY KEY,
    agent_id TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    system_prompt TEXT,
    model TEXT,
    bound_channels JSONB NOT NULL DEFAULT '[]'::jsonb,
    trigger_keywords JSONB NOT NULL DEFAULT '[]'::jsonb,
    allowed_tools JSONB,
    allowed_skills JSONB,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE agent_workspaces
    ADD COLUMN IF NOT EXISTS allowed_tools JSONB;

ALTER TABLE agent_workspaces
    ADD COLUMN IF NOT EXISTS allowed_skills JSONB;
