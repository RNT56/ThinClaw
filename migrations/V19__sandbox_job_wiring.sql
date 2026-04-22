ALTER TABLE agent_jobs
ADD COLUMN IF NOT EXISTS credential_grants TEXT NOT NULL DEFAULT '[]';

UPDATE agent_jobs
SET credential_grants = COALESCE(NULLIF(description, ''), '[]')
WHERE source = 'sandbox'
  AND COALESCE(NULLIF(credential_grants, ''), '[]') = '[]'
  AND description LIKE '[%';
