ALTER TABLE agent_workspaces
    ADD COLUMN IF NOT EXISTS allowed_tools JSONB;

ALTER TABLE agent_workspaces
    ADD COLUMN IF NOT EXISTS allowed_skills JSONB;
