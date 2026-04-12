-- Learning ledger and transcript search foundation.
--
-- Adds durable storage for learning events and related artifacts, plus
-- full-text transcript search over conversation_messages.content.

CREATE TABLE IF NOT EXISTS learning_events (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    actor_id TEXT,
    channel TEXT,
    thread_id TEXT,
    conversation_id UUID REFERENCES conversations(id) ON DELETE SET NULL,
    message_id UUID REFERENCES conversation_messages(id) ON DELETE SET NULL,
    job_id UUID REFERENCES agent_jobs(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    source TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_learning_events_user_created
    ON learning_events (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_events_actor_created
    ON learning_events (actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_events_channel_created
    ON learning_events (channel, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_events_thread_created
    ON learning_events (thread_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_events_conversation
    ON learning_events (conversation_id);
CREATE INDEX IF NOT EXISTS idx_learning_events_job
    ON learning_events (job_id);
CREATE INDEX IF NOT EXISTS idx_learning_events_event_type
    ON learning_events (event_type);

CREATE TABLE IF NOT EXISTS learning_evaluations (
    id UUID PRIMARY KEY,
    learning_event_id UUID NOT NULL REFERENCES learning_events(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    evaluator TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    score DOUBLE PRECISION,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_learning_evaluations_event
    ON learning_evaluations (learning_event_id);
CREATE INDEX IF NOT EXISTS idx_learning_evaluations_user_created
    ON learning_evaluations (user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS learning_candidates (
    id UUID PRIMARY KEY,
    learning_event_id UUID REFERENCES learning_events(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    candidate_type TEXT NOT NULL,
    risk_tier TEXT NOT NULL,
    confidence DOUBLE PRECISION,
    target_type TEXT,
    target_name TEXT,
    summary TEXT,
    proposal JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_learning_candidates_event
    ON learning_candidates (learning_event_id);
CREATE INDEX IF NOT EXISTS idx_learning_candidates_user_created
    ON learning_candidates (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_candidates_type
    ON learning_candidates (candidate_type);

CREATE TABLE IF NOT EXISTS learning_artifact_versions (
    id UUID PRIMARY KEY,
    candidate_id UUID REFERENCES learning_candidates(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    artifact_type TEXT NOT NULL,
    artifact_name TEXT NOT NULL,
    version_label TEXT,
    status TEXT NOT NULL,
    diff_summary TEXT,
    before_content TEXT,
    after_content TEXT,
    provenance JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_learning_artifact_versions_candidate
    ON learning_artifact_versions (candidate_id);
CREATE INDEX IF NOT EXISTS idx_learning_artifact_versions_user_created
    ON learning_artifact_versions (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_artifact_versions_artifact
    ON learning_artifact_versions (artifact_type, artifact_name);

CREATE TABLE IF NOT EXISTS learning_feedback (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    verdict TEXT NOT NULL,
    note TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_learning_feedback_user_created
    ON learning_feedback (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_feedback_target
    ON learning_feedback (target_type, target_id);

CREATE TABLE IF NOT EXISTS learning_rollbacks (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    artifact_type TEXT NOT NULL,
    artifact_name TEXT NOT NULL,
    artifact_version_id UUID,
    reason TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_learning_rollbacks_user_created
    ON learning_rollbacks (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_rollbacks_artifact
    ON learning_rollbacks (artifact_type, artifact_name);

CREATE TABLE IF NOT EXISTS learning_code_proposals (
    id UUID PRIMARY KEY,
    learning_event_id UUID REFERENCES learning_events(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'proposed',
    title TEXT NOT NULL,
    rationale TEXT NOT NULL,
    target_files JSONB NOT NULL DEFAULT '[]'::jsonb,
    diff TEXT NOT NULL DEFAULT '',
    validation_results JSONB NOT NULL DEFAULT '{}'::jsonb,
    rollback_note TEXT,
    confidence DOUBLE PRECISION,
    branch_name TEXT,
    pr_url TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_learning_code_proposals_event
    ON learning_code_proposals (learning_event_id);
CREATE INDEX IF NOT EXISTS idx_learning_code_proposals_user_status_created
    ON learning_code_proposals (user_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_code_proposals_status
    ON learning_code_proposals (status);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_conversation_created_at
    ON conversation_messages (conversation_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_actor_created_at
    ON conversation_messages (actor_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_content_fts
    ON conversation_messages
    USING GIN (to_tsvector('simple', COALESCE(content, '')));
