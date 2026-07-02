-- Durable ledger for in-process sub-agent runs (SubagentExecutor).
--
-- Running sub-agents previously lived only in an in-memory map
-- (`SubagentExecutor::active`), so a process restart silently dropped
-- in-flight delegated work — including any routine run a sub-agent was
-- finalizing. This table gives the executor a durable record it writes on
-- spawn and updates on completion, plus enough information for a startup
-- reconciliation pass to fail orphaned rows left `running` by a crash.

CREATE TABLE IF NOT EXISTS subagent_runs (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    task TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',
    parent_thread_id TEXT,
    routine_run_id TEXT,
    spawned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_subagent_runs_status ON subagent_runs(status);
CREATE INDEX IF NOT EXISTS idx_subagent_runs_routine_run ON subagent_runs(routine_run_id);
