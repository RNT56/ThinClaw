ALTER TABLE agent_jobs
    ADD COLUMN IF NOT EXISTS principal_id TEXT NOT NULL DEFAULT 'default',
    ADD COLUMN IF NOT EXISTS total_tokens_used BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS max_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS transitions JSONB NOT NULL DEFAULT '[]'::jsonb;

UPDATE agent_jobs
SET principal_id = user_id
WHERE user_id IS NOT NULL
  AND principal_id = 'default';
