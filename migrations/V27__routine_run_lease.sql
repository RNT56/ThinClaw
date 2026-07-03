-- Renewable lease for routine runs, replacing the fixed 10-minute zombie TTL.
--
-- Full-job routine runs legitimately stay `running` for up to AGENT_JOB_TIMEOUT
-- (default 3600s). The old zombie reaper marked any `running` row older than a
-- hardcoded 10 minutes as failed, which falsely orphaned long-running jobs.
--
-- Workers and subagents now renew this lease periodically while actively
-- executing a routine run. The reaper only reaps runs whose lease has
-- expired; legacy rows with a NULL lease fall back to a conservative TTL
-- supplied by the caller (default 3600s) instead of the old 10-minute cutoff.

ALTER TABLE routine_runs
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_routine_runs_lease_expires
    ON routine_runs (lease_expires_at)
    WHERE status = 'running';
