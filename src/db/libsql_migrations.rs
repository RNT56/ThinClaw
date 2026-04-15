//! SQLite-dialect migrations for the libSQL/Turso backend.
//!
//! Consolidates all PostgreSQL migrations (V1-V8) into a single SQLite-compatible
//! schema. Run once on database creation; idempotent via `IF NOT EXISTS`.

/// Consolidated schema for libSQL.
///
/// Translates PostgreSQL types and features:
/// - `UUID` -> `TEXT` (store as hex string)
/// - `TIMESTAMPTZ` -> `TEXT` (ISO-8601)
/// - `JSONB` -> `TEXT` (JSON encoded)
/// - `BYTEA` -> `BLOB`
/// - `NUMERIC` -> `TEXT` (preserve precision for rust_decimal)
/// - `TEXT[]` -> `TEXT` (JSON array)
/// - `VECTOR(1536)` -> `F32_BLOB(1536)` plus canonical `BLOB` + `dim`
/// - `TSVECTOR` -> FTS5 virtual table
/// - `BIGSERIAL` -> `INTEGER PRIMARY KEY AUTOINCREMENT`
/// - PL/pgSQL functions -> SQLite triggers
pub const SCHEMA: &str = r#"

-- ==================== Migration tracking ====================

CREATE TABLE IF NOT EXISTS _migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ==================== Conversations ====================

CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    user_id TEXT NOT NULL,
    actor_id TEXT,
    conversation_scope_id TEXT,
    conversation_kind TEXT NOT NULL DEFAULT 'direct',
    thread_id TEXT,
    stable_external_conversation_key TEXT,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_activity TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_conversations_channel ON conversations(channel);
CREATE INDEX IF NOT EXISTS idx_conversations_user ON conversations(user_id);
CREATE INDEX IF NOT EXISTS idx_conversations_actor ON conversations(actor_id);
CREATE INDEX IF NOT EXISTS idx_conversations_scope ON conversations(conversation_scope_id);
CREATE INDEX IF NOT EXISTS idx_conversations_last_activity ON conversations(last_activity);

CREATE TABLE IF NOT EXISTS conversation_messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    actor_id TEXT,
    actor_display_name TEXT,
    raw_sender_id TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_conversation
    ON conversation_messages(conversation_id);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_conversation_created_at
    ON conversation_messages(conversation_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_actor_created_at
    ON conversation_messages(actor_id, created_at DESC);

-- FTS5 search index for transcript lookup.
CREATE VIRTUAL TABLE IF NOT EXISTS conversation_messages_fts USING fts5(
    content,
    content='conversation_messages',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS conversation_messages_fts_insert
    AFTER INSERT ON conversation_messages
    FOR EACH ROW
    BEGIN
        INSERT INTO conversation_messages_fts(rowid, content)
            VALUES (new.rowid, new.content);
    END;

CREATE TRIGGER IF NOT EXISTS conversation_messages_fts_delete
    AFTER DELETE ON conversation_messages
    FOR EACH ROW
    BEGIN
        INSERT INTO conversation_messages_fts(conversation_messages_fts, rowid, content)
            VALUES ('delete', old.rowid, old.content);
    END;

CREATE TRIGGER IF NOT EXISTS conversation_messages_fts_update
    AFTER UPDATE ON conversation_messages
    FOR EACH ROW
    BEGIN
        INSERT INTO conversation_messages_fts(conversation_messages_fts, rowid, content)
            VALUES ('delete', old.rowid, old.content);
        INSERT INTO conversation_messages_fts(rowid, content)
            VALUES (new.rowid, new.content);
    END;

-- ==================== Learning ====================

CREATE TABLE IF NOT EXISTS learning_events (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    actor_id TEXT,
    channel TEXT,
    thread_id TEXT,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    message_id TEXT REFERENCES conversation_messages(id) ON DELETE SET NULL,
    job_id TEXT REFERENCES agent_jobs(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    source TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT '{}',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_learning_events_user_created
    ON learning_events(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_events_actor_created
    ON learning_events(actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_events_channel_created
    ON learning_events(channel, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_events_thread_created
    ON learning_events(thread_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_events_conversation
    ON learning_events(conversation_id);
CREATE INDEX IF NOT EXISTS idx_learning_events_job
    ON learning_events(job_id);
CREATE INDEX IF NOT EXISTS idx_learning_events_event_type
    ON learning_events(event_type);

CREATE TABLE IF NOT EXISTS learning_evaluations (
    id TEXT PRIMARY KEY,
    learning_event_id TEXT NOT NULL REFERENCES learning_events(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    evaluator TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    score REAL,
    details TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_learning_evaluations_event
    ON learning_evaluations(learning_event_id);
CREATE INDEX IF NOT EXISTS idx_learning_evaluations_user_created
    ON learning_evaluations(user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS learning_candidates (
    id TEXT PRIMARY KEY,
    learning_event_id TEXT REFERENCES learning_events(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    candidate_type TEXT NOT NULL,
    risk_tier TEXT NOT NULL,
    confidence REAL,
    target_type TEXT,
    target_name TEXT,
    summary TEXT,
    proposal TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_learning_candidates_event
    ON learning_candidates(learning_event_id);
CREATE INDEX IF NOT EXISTS idx_learning_candidates_user_created
    ON learning_candidates(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_candidates_type
    ON learning_candidates(candidate_type);

CREATE TABLE IF NOT EXISTS learning_artifact_versions (
    id TEXT PRIMARY KEY,
    candidate_id TEXT REFERENCES learning_candidates(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    artifact_type TEXT NOT NULL,
    artifact_name TEXT NOT NULL,
    version_label TEXT,
    status TEXT NOT NULL,
    diff_summary TEXT,
    before_content TEXT,
    after_content TEXT,
    provenance TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_learning_artifact_versions_candidate
    ON learning_artifact_versions(candidate_id);
CREATE INDEX IF NOT EXISTS idx_learning_artifact_versions_user_created
    ON learning_artifact_versions(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_artifact_versions_artifact
    ON learning_artifact_versions(artifact_type, artifact_name);

CREATE TABLE IF NOT EXISTS learning_feedback (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    verdict TEXT NOT NULL,
    note TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_learning_feedback_user_created
    ON learning_feedback(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_feedback_target
    ON learning_feedback(target_type, target_id);

CREATE TABLE IF NOT EXISTS learning_rollbacks (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    artifact_type TEXT NOT NULL,
    artifact_name TEXT NOT NULL,
    artifact_version_id TEXT,
    reason TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_learning_rollbacks_user_created
    ON learning_rollbacks(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_rollbacks_artifact
    ON learning_rollbacks(artifact_type, artifact_name);

CREATE TABLE IF NOT EXISTS learning_code_proposals (
    id TEXT PRIMARY KEY,
    learning_event_id TEXT REFERENCES learning_events(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'proposed',
    title TEXT NOT NULL,
    rationale TEXT NOT NULL,
    target_files TEXT NOT NULL DEFAULT '[]',
    diff TEXT NOT NULL DEFAULT '',
    validation_results TEXT NOT NULL DEFAULT '{}',
    rollback_note TEXT,
    confidence REAL,
    branch_name TEXT,
    pr_url TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_learning_code_proposals_event
    ON learning_code_proposals(learning_event_id);
CREATE INDEX IF NOT EXISTS idx_learning_code_proposals_user_status_created
    ON learning_code_proposals(user_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_learning_code_proposals_status
    ON learning_code_proposals(status);

CREATE TABLE IF NOT EXISTS outcome_contracts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    actor_id TEXT,
    channel TEXT,
    thread_id TEXT,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    contract_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    summary TEXT,
    due_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    final_verdict TEXT,
    final_score REAL,
    evaluation_details TEXT NOT NULL DEFAULT '{}',
    metadata TEXT NOT NULL DEFAULT '{}',
    dedupe_key TEXT NOT NULL,
    claimed_at TEXT,
    evaluated_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_outcome_contracts_dedupe_key
    ON outcome_contracts(dedupe_key);
CREATE INDEX IF NOT EXISTS idx_outcome_contracts_status_due
    ON outcome_contracts(status, due_at);
CREATE INDEX IF NOT EXISTS idx_outcome_contracts_user_actor_thread_status
    ON outcome_contracts(user_id, actor_id, thread_id, status);
CREATE INDEX IF NOT EXISTS idx_outcome_contracts_source
    ON outcome_contracts(source_kind, source_id);

CREATE TABLE IF NOT EXISTS outcome_observations (
    id TEXT PRIMARY KEY,
    contract_id TEXT NOT NULL REFERENCES outcome_contracts(id) ON DELETE CASCADE,
    observation_kind TEXT NOT NULL,
    polarity TEXT NOT NULL,
    weight REAL NOT NULL DEFAULT 0,
    summary TEXT,
    evidence TEXT NOT NULL DEFAULT '{}',
    fingerprint TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_outcome_observations_contract_fingerprint
    ON outcome_observations(contract_id, fingerprint);
CREATE INDEX IF NOT EXISTS idx_outcome_observations_contract_observed_at
    ON outcome_observations(contract_id, observed_at DESC);

-- ==================== Experiments ====================

CREATE TABLE IF NOT EXISTS experiment_runner_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    backend TEXT NOT NULL,
    backend_config TEXT NOT NULL DEFAULT '{}',
    image_or_runtime TEXT,
    gpu_requirements TEXT NOT NULL DEFAULT '{}',
    env_grants TEXT NOT NULL DEFAULT '{}',
    secret_references TEXT NOT NULL DEFAULT '[]',
    cache_policy TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_experiment_runner_profiles_updated
    ON experiment_runner_profiles(updated_at DESC);

CREATE TABLE IF NOT EXISTS experiment_projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    git_remote_name TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    preset TEXT NOT NULL DEFAULT 'autoresearch_single_file',
    strategy_prompt TEXT NOT NULL DEFAULT '',
    workdir TEXT NOT NULL DEFAULT '.',
    prepare_command TEXT,
    run_command TEXT NOT NULL,
    mutable_paths TEXT NOT NULL DEFAULT '[]',
    fixed_paths TEXT NOT NULL DEFAULT '[]',
    primary_metric TEXT NOT NULL DEFAULT '{}',
    secondary_metrics TEXT NOT NULL DEFAULT '[]',
    comparison_policy TEXT NOT NULL DEFAULT '{}',
    stop_policy TEXT NOT NULL DEFAULT '{}',
    default_runner_profile_id TEXT REFERENCES experiment_runner_profiles(id) ON DELETE SET NULL,
    promotion_mode TEXT NOT NULL DEFAULT 'branch_pr_draft',
    autonomy_mode TEXT NOT NULL DEFAULT 'autonomous',
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_experiment_projects_updated
    ON experiment_projects(updated_at DESC);

CREATE TABLE IF NOT EXISTS experiment_campaigns (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES experiment_projects(id) ON DELETE CASCADE,
    runner_profile_id TEXT NOT NULL REFERENCES experiment_runner_profiles(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'pending_baseline',
    baseline_commit TEXT,
    best_commit TEXT,
    best_metrics TEXT NOT NULL DEFAULT '{}',
    experiment_branch TEXT,
    remote_ref TEXT,
    worktree_path TEXT,
    started_at TEXT,
    ended_at TEXT,
    trial_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    pause_reason TEXT,
    queue_state TEXT NOT NULL DEFAULT 'not_queued',
    queue_position INTEGER NOT NULL DEFAULT 0,
    active_trial_id TEXT,
    total_runtime_ms INTEGER NOT NULL DEFAULT 0,
    total_cost_usd TEXT NOT NULL DEFAULT '0',
    consecutive_non_improving_trials INTEGER NOT NULL DEFAULT 0,
    max_trials_override INTEGER,
    gateway_url TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    total_llm_cost_usd TEXT NOT NULL DEFAULT '0',
    total_runner_cost_usd TEXT NOT NULL DEFAULT '0'
);

CREATE INDEX IF NOT EXISTS idx_experiment_campaigns_project
    ON experiment_campaigns(project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS experiment_trials (
    id TEXT PRIMARY KEY,
    campaign_id TEXT NOT NULL REFERENCES experiment_campaigns(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    candidate_commit TEXT,
    parent_best_commit TEXT,
    status TEXT NOT NULL DEFAULT 'preparing',
    runner_backend TEXT NOT NULL,
    exit_code INTEGER,
    metrics_json TEXT NOT NULL DEFAULT '{}',
    summary TEXT,
    decision_reason TEXT,
    log_preview_path TEXT,
    artifact_manifest_json TEXT NOT NULL DEFAULT '{}',
    runtime_ms INTEGER,
    attributed_cost_usd TEXT,
    hypothesis TEXT,
    mutation_summary TEXT,
    reviewer_decision TEXT,
    provider_job_id TEXT,
    provider_job_metadata TEXT NOT NULL DEFAULT '{}',
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    llm_cost_usd TEXT,
    runner_cost_usd TEXT,
    UNIQUE (campaign_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_experiment_trials_campaign
    ON experiment_trials(campaign_id, sequence ASC);

CREATE TABLE IF NOT EXISTS experiment_artifact_refs (
    id TEXT PRIMARY KEY,
    trial_id TEXT NOT NULL REFERENCES experiment_trials(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    uri_or_local_path TEXT NOT NULL,
    size_bytes INTEGER,
    fetchable INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_experiment_artifact_refs_trial
    ON experiment_artifact_refs(trial_id, created_at ASC);

CREATE TABLE IF NOT EXISTS experiment_leases (
    id TEXT PRIMARY KEY,
    campaign_id TEXT NOT NULL REFERENCES experiment_campaigns(id) ON DELETE CASCADE,
    trial_id TEXT NOT NULL REFERENCES experiment_trials(id) ON DELETE CASCADE,
    runner_profile_id TEXT NOT NULL REFERENCES experiment_runner_profiles(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending',
    token_hash TEXT NOT NULL,
    job_payload TEXT NOT NULL DEFAULT '{}',
    credentials_payload TEXT NOT NULL DEFAULT '{}',
    expires_at TEXT NOT NULL,
    claimed_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_experiment_leases_trial
    ON experiment_leases(trial_id, created_at DESC);

CREATE TABLE IF NOT EXISTS experiment_targets (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT '"prompt_asset"',
    location TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_experiment_targets_updated
    ON experiment_targets(updated_at DESC, name ASC);

CREATE TABLE IF NOT EXISTS experiment_target_links (
    id TEXT PRIMARY KEY,
    target_id TEXT NOT NULL REFERENCES experiment_targets(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT '"prompt_asset"',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    route_key TEXT NOT NULL DEFAULT '',
    logical_role TEXT NOT NULL DEFAULT '',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (target_id, kind, provider, model, route_key, logical_role)
);

CREATE INDEX IF NOT EXISTS idx_experiment_target_links_lookup
    ON experiment_target_links(provider, model, updated_at DESC);

CREATE TABLE IF NOT EXISTS experiment_model_usage_records (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    route_key TEXT,
    logical_role TEXT,
    endpoint_type TEXT,
    workload_tag TEXT,
    latency_ms INTEGER,
    cost_usd TEXT,
    success INTEGER NOT NULL DEFAULT 1,
    prompt_asset_ids TEXT NOT NULL DEFAULT '[]',
    retrieval_asset_ids TEXT NOT NULL DEFAULT '[]',
    tool_policy_ids TEXT NOT NULL DEFAULT '[]',
    evaluator_ids TEXT NOT NULL DEFAULT '[]',
    parser_ids TEXT NOT NULL DEFAULT '[]',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_experiment_model_usage_created
    ON experiment_model_usage_records(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_experiment_model_usage_provider_model
    ON experiment_model_usage_records(provider, model, created_at DESC);

-- ==================== Identity Registry ====================

CREATE TABLE IF NOT EXISTS actors (
    actor_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL DEFAULT 'default',
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    preferred_delivery_channel TEXT,
    preferred_delivery_external_user_id TEXT,
    last_active_direct_channel TEXT,
    last_active_direct_external_user_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (
        (preferred_delivery_channel IS NULL) = (preferred_delivery_external_user_id IS NULL)
    ),
    CHECK (
        (last_active_direct_channel IS NULL) = (last_active_direct_external_user_id IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_actors_principal ON actors(principal_id);
CREATE INDEX IF NOT EXISTS idx_actors_status ON actors(status);

CREATE TABLE IF NOT EXISTS actor_endpoints (
    channel TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    actor_id TEXT NOT NULL REFERENCES actors(actor_id) ON DELETE CASCADE,
    endpoint_metadata TEXT NOT NULL DEFAULT '{}',
    approval_status TEXT NOT NULL DEFAULT 'approved',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (channel, external_user_id)
);

CREATE INDEX IF NOT EXISTS idx_actor_endpoints_actor_id ON actor_endpoints(actor_id);
CREATE INDEX IF NOT EXISTS idx_actor_endpoints_status ON actor_endpoints(approval_status);

-- ==================== Agent Jobs ====================

CREATE TABLE IF NOT EXISTS agent_jobs (
    id TEXT PRIMARY KEY,
    marketplace_job_id TEXT,
    conversation_id TEXT REFERENCES conversations(id),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    user_id TEXT NOT NULL DEFAULT 'default',
    principal_id TEXT NOT NULL DEFAULT 'default',
    actor_id TEXT NOT NULL DEFAULT 'default',
    project_dir TEXT,
    job_mode TEXT NOT NULL DEFAULT 'worker',
    budget_amount TEXT,
    budget_token TEXT,
    bid_amount TEXT,
    estimated_cost TEXT,
    estimated_time_secs INTEGER,
    estimated_value TEXT,
    actual_cost TEXT,
    actual_time_secs INTEGER,
    success INTEGER,
    failure_reason TEXT,
    stuck_since TEXT,
    total_tokens_used INTEGER NOT NULL DEFAULT 0,
    max_tokens INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    transitions TEXT NOT NULL DEFAULT '[]',
    repair_attempts INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    started_at TEXT,
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_jobs_status ON agent_jobs(status);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_marketplace ON agent_jobs(marketplace_job_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_conversation ON agent_jobs(conversation_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_source ON agent_jobs(source);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_user ON agent_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_actor ON agent_jobs(actor_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_created ON agent_jobs(created_at DESC);

CREATE TABLE IF NOT EXISTS job_actions (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
    sequence_num INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    input TEXT NOT NULL,
    output_raw TEXT,
    output_sanitized TEXT,
    sanitization_warnings TEXT,
    cost TEXT,
    duration_ms INTEGER,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(job_id, sequence_num)
);

CREATE INDEX IF NOT EXISTS idx_job_actions_job_id ON job_actions(job_id);
CREATE INDEX IF NOT EXISTS idx_job_actions_tool ON job_actions(tool_name);

-- ==================== Dynamic Tools ====================

CREATE TABLE IF NOT EXISTS dynamic_tools (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    parameters_schema TEXT NOT NULL,
    code TEXT NOT NULL,
    sandbox_config TEXT NOT NULL,
    created_by_job_id TEXT REFERENCES agent_jobs(id),
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_dynamic_tools_status ON dynamic_tools(status);
CREATE INDEX IF NOT EXISTS idx_dynamic_tools_name ON dynamic_tools(name);

-- ==================== LLM Calls ====================

CREATE TABLE IF NOT EXISTS llm_calls (
    id TEXT PRIMARY KEY,
    job_id TEXT REFERENCES agent_jobs(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES conversations(id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost TEXT NOT NULL,
    purpose TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_llm_calls_job ON llm_calls(job_id);
CREATE INDEX IF NOT EXISTS idx_llm_calls_conversation ON llm_calls(conversation_id);
CREATE INDEX IF NOT EXISTS idx_llm_calls_provider ON llm_calls(provider);

-- ==================== Estimation ====================

CREATE TABLE IF NOT EXISTS estimation_snapshots (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    tool_names TEXT NOT NULL DEFAULT '[]',
    estimated_cost TEXT NOT NULL,
    actual_cost TEXT,
    estimated_time_secs INTEGER NOT NULL,
    actual_time_secs INTEGER,
    estimated_value TEXT NOT NULL,
    actual_value TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_estimation_category ON estimation_snapshots(category);
CREATE INDEX IF NOT EXISTS idx_estimation_job ON estimation_snapshots(job_id);

-- ==================== Self Repair ====================

CREATE TABLE IF NOT EXISTS repair_attempts (
    id TEXT PRIMARY KEY,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    diagnosis TEXT NOT NULL,
    action_taken TEXT NOT NULL,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_repair_attempts_target ON repair_attempts(target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_repair_attempts_created ON repair_attempts(created_at);

-- ==================== Workspace: Memory Documents ====================

CREATE TABLE IF NOT EXISTS memory_documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT NOT NULL DEFAULT '{}',
    UNIQUE (user_id, agent_id, path)
);

CREATE INDEX IF NOT EXISTS idx_memory_documents_user ON memory_documents(user_id);
CREATE INDEX IF NOT EXISTS idx_memory_documents_path ON memory_documents(user_id, path);
CREATE INDEX IF NOT EXISTS idx_memory_documents_updated ON memory_documents(updated_at DESC);

-- Trigger to auto-update updated_at on memory_documents
CREATE TRIGGER IF NOT EXISTS update_memory_documents_updated_at
    AFTER UPDATE ON memory_documents
    FOR EACH ROW
    WHEN NEW.updated_at = OLD.updated_at
    BEGIN
        UPDATE memory_documents SET updated_at = datetime('now') WHERE id = NEW.id;
    END;

-- ==================== Workspace: Memory Chunks ====================

CREATE TABLE IF NOT EXISTS memory_chunks (
    _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding F32_BLOB(1536),
    embedding_blob BLOB,
    embedding_dim INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);
CREATE INDEX IF NOT EXISTS idx_memory_chunks_embedding_dim ON memory_chunks(embedding_dim);

-- Vector index for semantic search (libSQL native)
CREATE INDEX IF NOT EXISTS idx_memory_chunks_embedding
    ON memory_chunks (libsql_vector_idx(embedding));

-- FTS5 virtual table for full-text search
CREATE VIRTUAL TABLE IF NOT EXISTS memory_chunks_fts USING fts5(
    content,
    content='memory_chunks',
    content_rowid='_rowid'
);

-- Triggers to keep FTS5 in sync with memory_chunks
CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_insert AFTER INSERT ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_delete AFTER DELETE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_update AFTER UPDATE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;

-- ==================== Workspace: Heartbeat State ====================

CREATE TABLE IF NOT EXISTS heartbeat_state (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    last_run TEXT,
    next_run TEXT,
    interval_seconds INTEGER NOT NULL DEFAULT 1800,
    enabled INTEGER NOT NULL DEFAULT 1,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_checks TEXT NOT NULL DEFAULT '{}',
    UNIQUE (user_id, agent_id)
);

CREATE INDEX IF NOT EXISTS idx_heartbeat_user ON heartbeat_state(user_id);

-- ==================== Secrets ====================

CREATE TABLE IF NOT EXISTS secrets (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    encrypted_value BLOB NOT NULL,
    key_salt BLOB NOT NULL,
    provider TEXT,
    expires_at TEXT,
    last_used_at TEXT,
    usage_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_secrets_user ON secrets(user_id);

-- ==================== WASM Tools ====================

CREATE TABLE IF NOT EXISTS wasm_tools (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',
    description TEXT NOT NULL,
    wasm_binary BLOB NOT NULL,
    binary_hash BLOB NOT NULL,
    parameters_schema TEXT NOT NULL,
    source_url TEXT,
    trust_level TEXT NOT NULL DEFAULT 'user',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (user_id, name, version)
);

CREATE INDEX IF NOT EXISTS idx_wasm_tools_user ON wasm_tools(user_id);
CREATE INDEX IF NOT EXISTS idx_wasm_tools_name ON wasm_tools(user_id, name);
CREATE INDEX IF NOT EXISTS idx_wasm_tools_status ON wasm_tools(status);

-- ==================== Tool Capabilities ====================

CREATE TABLE IF NOT EXISTS tool_capabilities (
    id TEXT PRIMARY KEY,
    wasm_tool_id TEXT NOT NULL REFERENCES wasm_tools(id) ON DELETE CASCADE,
    http_allowlist TEXT NOT NULL DEFAULT '[]',
    allowed_secrets TEXT NOT NULL DEFAULT '[]',
    tool_aliases TEXT NOT NULL DEFAULT '{}',
    requests_per_minute INTEGER NOT NULL DEFAULT 60,
    requests_per_hour INTEGER NOT NULL DEFAULT 1000,
    max_request_body_bytes INTEGER NOT NULL DEFAULT 1048576,
    max_response_body_bytes INTEGER NOT NULL DEFAULT 10485760,
    workspace_read_prefixes TEXT NOT NULL DEFAULT '[]',
    http_timeout_secs INTEGER NOT NULL DEFAULT 30,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (wasm_tool_id)
);

-- ==================== Leak Detection Patterns ====================

CREATE TABLE IF NOT EXISTS leak_detection_patterns (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    pattern TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'high',
    action TEXT NOT NULL DEFAULT 'block',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ==================== Rate Limit State ====================

CREATE TABLE IF NOT EXISTS tool_rate_limit_state (
    id TEXT PRIMARY KEY,
    wasm_tool_id TEXT NOT NULL REFERENCES wasm_tools(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    minute_window_start TEXT NOT NULL DEFAULT (datetime('now')),
    minute_count INTEGER NOT NULL DEFAULT 0,
    hour_window_start TEXT NOT NULL DEFAULT (datetime('now')),
    hour_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE (wasm_tool_id, user_id)
);

-- ==================== Secret Usage Audit Log ====================

CREATE TABLE IF NOT EXISTS secret_usage_log (
    id TEXT PRIMARY KEY,
    secret_id TEXT NOT NULL REFERENCES secrets(id) ON DELETE CASCADE,
    wasm_tool_id TEXT REFERENCES wasm_tools(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    target_host TEXT NOT NULL,
    target_path TEXT,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_secret_usage_user ON secret_usage_log(user_id);

-- ==================== Leak Detection Events ====================

CREATE TABLE IF NOT EXISTS leak_detection_events (
    id TEXT PRIMARY KEY,
    pattern_id TEXT REFERENCES leak_detection_patterns(id) ON DELETE SET NULL,
    wasm_tool_id TEXT REFERENCES wasm_tools(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    source TEXT NOT NULL,
    action_taken TEXT NOT NULL,
    context_preview TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ==================== Tool Failures ====================

CREATE TABLE IF NOT EXISTS tool_failures (
    id TEXT PRIMARY KEY,
    tool_name TEXT NOT NULL UNIQUE,
    error_message TEXT,
    error_count INTEGER DEFAULT 1,
    first_failure TEXT DEFAULT (datetime('now')),
    last_failure TEXT DEFAULT (datetime('now')),
    last_build_result TEXT,
    repaired_at TEXT,
    repair_attempts INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_tool_failures_name ON tool_failures(tool_name);

-- ==================== Job Events ====================

CREATE TABLE IF NOT EXISTS job_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id),
    event_type TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_job_events_job ON job_events(job_id, id);

-- ==================== Routines ====================

CREATE TABLE IF NOT EXISTS routines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL,
    actor_id TEXT NOT NULL DEFAULT 'default',
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_type TEXT NOT NULL,
    trigger_config TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_config TEXT NOT NULL,
    cooldown_secs INTEGER NOT NULL DEFAULT 300,
    max_concurrent INTEGER NOT NULL DEFAULT 1,
    dedup_window_secs INTEGER,
    notify_channel TEXT,
    notify_user TEXT NOT NULL DEFAULT 'default',
    notify_on_success INTEGER NOT NULL DEFAULT 0,
    notify_on_failure INTEGER NOT NULL DEFAULT 1,
    notify_on_attention INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT '{}',
    last_run_at TEXT,
    next_fire_at TEXT,
    run_count INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (user_id, actor_id, name)
);

CREATE INDEX IF NOT EXISTS idx_routines_user ON routines(user_id);
CREATE INDEX IF NOT EXISTS idx_routines_actor ON routines(actor_id);

-- ==================== Routine Runs ====================

CREATE TABLE IF NOT EXISTS routine_runs (
    id TEXT PRIMARY KEY,
    routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    trigger_type TEXT NOT NULL,
    trigger_detail TEXT,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT,
    status TEXT NOT NULL DEFAULT 'running',
    result_summary TEXT,
    tokens_used INTEGER,
    job_id TEXT REFERENCES agent_jobs(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_routine_runs_routine ON routine_runs(routine_id);

-- ==================== Settings ====================

CREATE TABLE IF NOT EXISTS settings (
    user_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, key)
);

CREATE INDEX IF NOT EXISTS idx_settings_user ON settings(user_id);

-- ==================== Missing indexes (parity with PostgreSQL) ====================

-- agent_jobs
CREATE INDEX IF NOT EXISTS idx_agent_jobs_stuck ON agent_jobs(stuck_since);

-- secrets
CREATE INDEX IF NOT EXISTS idx_secrets_provider ON secrets(provider);
CREATE INDEX IF NOT EXISTS idx_secrets_expires ON secrets(expires_at);

-- wasm_tools
CREATE INDEX IF NOT EXISTS idx_wasm_tools_trust ON wasm_tools(trust_level);

-- tool_capabilities
CREATE INDEX IF NOT EXISTS idx_tool_capabilities_tool ON tool_capabilities(wasm_tool_id);

-- leak_detection_patterns
CREATE INDEX IF NOT EXISTS idx_leak_patterns_enabled ON leak_detection_patterns(enabled);

-- tool_rate_limit_state
CREATE INDEX IF NOT EXISTS idx_rate_limit_tool ON tool_rate_limit_state(wasm_tool_id);

-- secret_usage_log
CREATE INDEX IF NOT EXISTS idx_secret_usage_secret ON secret_usage_log(secret_id);
CREATE INDEX IF NOT EXISTS idx_secret_usage_tool ON secret_usage_log(wasm_tool_id);
CREATE INDEX IF NOT EXISTS idx_secret_usage_created ON secret_usage_log(created_at DESC);

-- leak_detection_events
CREATE INDEX IF NOT EXISTS idx_leak_events_pattern ON leak_detection_events(pattern_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_tool ON leak_detection_events(wasm_tool_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_user ON leak_detection_events(user_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_created ON leak_detection_events(created_at DESC);

-- tool_failures
CREATE INDEX IF NOT EXISTS idx_tool_failures_count ON tool_failures(error_count DESC);
CREATE INDEX IF NOT EXISTS idx_tool_failures_unrepaired ON tool_failures(tool_name);

-- routines
CREATE INDEX IF NOT EXISTS idx_routines_next_fire ON routines(next_fire_at);
CREATE INDEX IF NOT EXISTS idx_routines_event_triggers ON routines(user_id);

-- routine_runs
CREATE INDEX IF NOT EXISTS idx_routine_runs_status ON routine_runs(status);

-- heartbeat_state
CREATE INDEX IF NOT EXISTS idx_heartbeat_next_run ON heartbeat_state(next_run);

-- ==================== Seed data ====================

-- Pre-populate leak detection patterns (matches PostgreSQL V2 migration).
INSERT OR IGNORE INTO leak_detection_patterns (id, name, pattern, severity, action, enabled, created_at) VALUES
    ('550e8400-e29b-41d4-a716-446655440001', 'openai_api_key', 'sk-(?:proj-)?[a-zA-Z0-9]{20,}(?:T3BlbkFJ[a-zA-Z0-9_-]*)?', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440002', 'anthropic_api_key', 'sk-ant-api[a-zA-Z0-9_-]{90,}', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440003', 'aws_access_key', 'AKIA[0-9A-Z]{16}', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440004', 'aws_secret_key', '(?<![A-Za-z0-9/+=])[A-Za-z0-9/+=]{40}(?![A-Za-z0-9/+=])', 'high', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440005', 'github_token', 'gh[pousr]_[A-Za-z0-9_]{36,}', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440006', 'github_fine_grained_pat', 'github_pat_[a-zA-Z0-9]{22}_[a-zA-Z0-9]{59}', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440007', 'stripe_api_key', 'sk_(?:live|test)_[a-zA-Z0-9]{24,}', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440008', 'nearai_session', 'sess_[a-zA-Z0-9]{32,}', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440009', 'bearer_token', 'Bearer\s+[a-zA-Z0-9_-]{20,}', 'high', 'redact', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-44665544000a', 'pem_private_key', '-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-44665544000b', 'ssh_private_key', '-----BEGIN\s+(?:OPENSSH|EC|DSA)\s+PRIVATE\s+KEY-----', 'critical', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-44665544000c', 'google_api_key', 'AIza[0-9A-Za-z_-]{35}', 'high', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-44665544000d', 'slack_token', 'xox[baprs]-[0-9a-zA-Z-]{10,}', 'high', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-44665544000e', 'discord_token', '[MN][A-Za-z\d]{23,}\.[\w-]{6}\.[\w-]{27}', 'high', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-44665544000f', 'twilio_api_key', 'SK[a-fA-F0-9]{32}', 'high', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440010', 'sendgrid_api_key', 'SG\.[a-zA-Z0-9_-]{22}\.[a-zA-Z0-9_-]{43}', 'high', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440011', 'mailchimp_api_key', '[a-f0-9]{32}-us[0-9]{1,2}', 'medium', 'block', 1, datetime('now')),
    ('550e8400-e29b-41d4-a716-446655440012', 'high_entropy_hex', '(?<![a-fA-F0-9])[a-fA-F0-9]{64}(?![a-fA-F0-9])', 'medium', 'warn', 1, datetime('now'));

-- ==================== Agent Workspaces ====================

CREATE TABLE IF NOT EXISTS agent_workspaces (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    system_prompt TEXT,
    model TEXT,
    bound_channels TEXT NOT NULL DEFAULT '[]',
    trigger_keywords TEXT NOT NULL DEFAULT '[]',
    allowed_tools TEXT,
    allowed_skills TEXT,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_agent_ws_agent_id ON agent_workspaces(agent_id);
CREATE INDEX IF NOT EXISTS idx_agent_ws_default ON agent_workspaces(is_default);

"#;

/// A single libSQL column upgrade step.
#[derive(Debug, Clone, Copy)]
pub struct LibsqlColumnUpgrade {
    /// Target migration version that this statement belongs to.
    pub version: u32,
    /// Human-readable reason for tracing and diagnostics.
    pub description: &'static str,
    /// SQL statement to execute.
    pub sql: &'static str,
}

/// Idempotent column-upgrade statements for existing libSQL databases.
///
/// The consolidated `SCHEMA` uses `CREATE TABLE IF NOT EXISTS`, which is a no-op
/// when the table already exists.  These `ALTER TABLE ADD COLUMN` statements
/// bring pre-V10/V11 tables up to date.  Each is run individually so that
/// "duplicate column" errors (column already present) can be safely ignored.
pub const UPGRADES: &[LibsqlColumnUpgrade] = &[
    // ── V11: conversations ──────────────────────────────────────────────
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add actor/scoping fields to conversations",
        sql: "ALTER TABLE conversations ADD COLUMN actor_id TEXT",
    },
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add conversation scope id",
        sql: "ALTER TABLE conversations ADD COLUMN conversation_scope_id TEXT",
    },
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add conversation kind field",
        sql: "ALTER TABLE conversations ADD COLUMN conversation_kind TEXT NOT NULL DEFAULT 'direct'",
    },
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add stable conversation key",
        sql: "ALTER TABLE conversations ADD COLUMN stable_external_conversation_key TEXT",
    },
    // ── V11: conversation_messages ───────────────────────────────────────
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add actor id to conversation messages",
        sql: "ALTER TABLE conversation_messages ADD COLUMN actor_id TEXT",
    },
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add actor display name to conversation messages",
        sql: "ALTER TABLE conversation_messages ADD COLUMN actor_display_name TEXT",
    },
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add raw sender id to conversation messages",
        sql: "ALTER TABLE conversation_messages ADD COLUMN raw_sender_id TEXT",
    },
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add metadata JSON to conversation messages",
        sql: "ALTER TABLE conversation_messages ADD COLUMN metadata TEXT NOT NULL DEFAULT '{}'",
    },
    // ── V10: actors + actor_endpoints (handled by CREATE TABLE IF NOT EXISTS) ──
    // ── V11: agent_jobs ─────────────────────────────────────────────────
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add legacy user fields to jobs",
        sql: "ALTER TABLE agent_jobs ADD COLUMN user_id TEXT NOT NULL DEFAULT 'default'",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add legacy principal actor reference to jobs",
        sql: "ALTER TABLE agent_jobs ADD COLUMN principal_id TEXT NOT NULL DEFAULT 'default'",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add actor id to jobs",
        sql: "ALTER TABLE agent_jobs ADD COLUMN actor_id TEXT NOT NULL DEFAULT 'default'",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add project directory to jobs",
        sql: "ALTER TABLE agent_jobs ADD COLUMN project_dir TEXT",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add job mode for worker behavior",
        sql: "ALTER TABLE agent_jobs ADD COLUMN job_mode TEXT NOT NULL DEFAULT 'worker'",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add job source metadata",
        sql: "ALTER TABLE agent_jobs ADD COLUMN source TEXT NOT NULL DEFAULT 'sandbox'",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add token usage counters",
        sql: "ALTER TABLE agent_jobs ADD COLUMN total_tokens_used INTEGER NOT NULL DEFAULT 0",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add max token budget",
        sql: "ALTER TABLE agent_jobs ADD COLUMN max_tokens INTEGER NOT NULL DEFAULT 0",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add structured job metadata",
        sql: "ALTER TABLE agent_jobs ADD COLUMN metadata TEXT NOT NULL DEFAULT '{}'",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Add job state transitions history",
        sql: "ALTER TABLE agent_jobs ADD COLUMN transitions TEXT NOT NULL DEFAULT '[]'",
    },
    LibsqlColumnUpgrade {
        version: 10,
        description: "Backfill principal id from user_id",
        sql: "UPDATE agent_jobs SET principal_id = user_id WHERE user_id IS NOT NULL AND (principal_id IS NULL OR principal_id = 'default')",
    },
    // ── V11: routines ───────────────────────────────────────────────────
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add actor id to routines",
        sql: "ALTER TABLE routines ADD COLUMN actor_id TEXT NOT NULL DEFAULT 'default'",
    },
    // ── V13: agent capability isolation ────────────────────────────────
    LibsqlColumnUpgrade {
        version: 13,
        description: "Add allowed tools/workspace restrictions",
        sql: "ALTER TABLE agent_workspaces ADD COLUMN allowed_tools TEXT",
    },
    LibsqlColumnUpgrade {
        version: 13,
        description: "Add allowed skills/workspace restrictions",
        sql: "ALTER TABLE agent_workspaces ADD COLUMN allowed_skills TEXT",
    },
    // ── V11: memory_chunks embedding parity ─────────────────────────────
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add binary embedding payload",
        sql: "ALTER TABLE memory_chunks ADD COLUMN embedding_blob BLOB",
    },
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add embedding dimension metadata",
        sql: "ALTER TABLE memory_chunks ADD COLUMN embedding_dim INTEGER",
    },
    LibsqlColumnUpgrade {
        version: 11,
        description: "Add index for embedding dimension",
        sql: "CREATE INDEX IF NOT EXISTS idx_memory_chunks_embedding_dim ON memory_chunks(embedding_dim)",
    },
    // ── V16: experiment campaign/trial persistence parity ──────────────
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add autonomy mode to experiment projects",
        sql: "ALTER TABLE experiment_projects ADD COLUMN autonomy_mode TEXT NOT NULL DEFAULT 'autonomous'",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add queue state to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN queue_state TEXT NOT NULL DEFAULT 'not_queued'",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add queue position to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN queue_position INTEGER NOT NULL DEFAULT 0",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add active trial id to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN active_trial_id TEXT",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add total runtime tracking to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN total_runtime_ms INTEGER NOT NULL DEFAULT 0",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add total cost tracking to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN total_cost_usd TEXT NOT NULL DEFAULT '0'",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add non-improving counter to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN consecutive_non_improving_trials INTEGER NOT NULL DEFAULT 0",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add trial override cap to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN max_trials_override INTEGER",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add campaign gateway override",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN gateway_url TEXT",
    },
    // ── V16: experiment trial metadata/tracing parity ─────────────────
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add runtime to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN runtime_ms INTEGER",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add attributed cost to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN attributed_cost_usd TEXT",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add hypothesis field to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN hypothesis TEXT",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add mutation summary to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN mutation_summary TEXT",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add reviewer decision to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN reviewer_decision TEXT",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add provider job id to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN provider_job_id TEXT",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add provider job metadata to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN provider_job_metadata TEXT NOT NULL DEFAULT '{}'",
    },
    // ── V16: experiment model usage attribution parity ─────────────────
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add evaluator ids to model usage records",
        sql: "ALTER TABLE experiment_model_usage_records ADD COLUMN evaluator_ids TEXT NOT NULL DEFAULT '[]'",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add parser ids to model usage records",
        sql: "ALTER TABLE experiment_model_usage_records ADD COLUMN parser_ids TEXT NOT NULL DEFAULT '[]'",
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Create experiment target links table",
        sql: r#"
            CREATE TABLE IF NOT EXISTS experiment_target_links (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL REFERENCES experiment_targets(id) ON DELETE CASCADE,
                kind TEXT NOT NULL DEFAULT '"prompt_asset"',
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                route_key TEXT NOT NULL DEFAULT '',
                logical_role TEXT NOT NULL DEFAULT '',
                metadata TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE (target_id, kind, provider, model, route_key, logical_role)
            )
        "#,
    },
    LibsqlColumnUpgrade {
        version: 16,
        description: "Add target-link lookup index",
        sql: "CREATE INDEX IF NOT EXISTS idx_experiment_target_links_lookup ON experiment_target_links(provider, model, updated_at DESC)",
    },
    LibsqlColumnUpgrade {
        version: 17,
        description: "Add total llm cost to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN total_llm_cost_usd TEXT NOT NULL DEFAULT '0'",
    },
    LibsqlColumnUpgrade {
        version: 17,
        description: "Add total runner cost to experiment campaigns",
        sql: "ALTER TABLE experiment_campaigns ADD COLUMN total_runner_cost_usd TEXT NOT NULL DEFAULT '0'",
    },
    LibsqlColumnUpgrade {
        version: 17,
        description: "Add llm cost to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN llm_cost_usd TEXT",
    },
    LibsqlColumnUpgrade {
        version: 17,
        description: "Add runner cost to experiment trials",
        sql: "ALTER TABLE experiment_trials ADD COLUMN runner_cost_usd TEXT",
    },
    LibsqlColumnUpgrade {
        version: 17,
        description: "Add model usage trial lookup index",
        sql: "CREATE INDEX IF NOT EXISTS idx_experiment_model_usage_trial ON experiment_model_usage_records(json_extract(metadata, '$.experiment_trial_id'), created_at DESC)",
    },
    LibsqlColumnUpgrade {
        version: 17,
        description: "Add model usage campaign lookup index",
        sql: "CREATE INDEX IF NOT EXISTS idx_experiment_model_usage_campaign ON experiment_model_usage_records(json_extract(metadata, '$.experiment_campaign_id'), created_at DESC)",
    },
];

/// Idempotent data repairs for legacy libSQL databases.
///
/// libSQL uses a consolidated schema rather than numbered per-version
/// migrations, so older deployments can pick up the new columns without
/// receiving the PostgreSQL-style backfill that populates them. Run these on
/// every startup after `SCHEMA` so null/empty legacy identity fields are
/// repaired in place.
pub const DATA_REPAIRS: &[&str] = &[
    r#"
    UPDATE conversations
    SET actor_id = COALESCE(NULLIF(actor_id, ''), user_id),
        conversation_scope_id = COALESCE(NULLIF(conversation_scope_id, ''), id),
        conversation_kind = COALESCE(NULLIF(conversation_kind, ''), 'direct'),
        stable_external_conversation_key = COALESCE(
            NULLIF(stable_external_conversation_key, ''),
            channel || ':' || COALESCE(NULLIF(thread_id, ''), id)
        )
    WHERE actor_id IS NULL
       OR actor_id = ''
       OR conversation_scope_id IS NULL
       OR conversation_scope_id = ''
       OR conversation_kind IS NULL
       OR conversation_kind = ''
       OR stable_external_conversation_key IS NULL
       OR stable_external_conversation_key = ''
    "#,
    r#"
    UPDATE agent_jobs
    SET principal_id = CASE
            WHEN principal_id IS NULL OR principal_id = '' OR principal_id = 'default'
                THEN COALESCE(user_id, 'default')
            ELSE principal_id
        END,
        actor_id = CASE
            WHEN actor_id IS NULL OR actor_id = '' OR actor_id = 'default'
                THEN COALESCE(user_id, NULLIF(principal_id, ''), 'default')
            ELSE actor_id
        END
    WHERE principal_id IS NULL
       OR principal_id = ''
       OR actor_id IS NULL
       OR actor_id = ''
       OR actor_id = 'default'
    "#,
    r#"
    UPDATE routines
    SET actor_id = CASE
            WHEN actor_id IS NULL OR actor_id = '' OR actor_id = 'default'
                THEN COALESCE(user_id, 'default')
            ELSE actor_id
        END
    WHERE actor_id IS NULL
       OR actor_id = ''
       OR actor_id = 'default'
    "#,
    r#"
    UPDATE memory_chunks
    SET embedding_blob = COALESCE(embedding_blob, embedding),
        embedding_dim = COALESCE(embedding_dim, 1536)
    WHERE embedding IS NOT NULL
      AND (embedding_blob IS NULL OR embedding_dim IS NULL)
    "#,
    r#"
    INSERT INTO conversation_messages_fts(conversation_messages_fts)
    VALUES ('rebuild')
    "#,
];

#[cfg(test)]
mod tests {
    use super::{DATA_REPAIRS, SCHEMA};

    #[test]
    fn schema_includes_learning_tables() {
        assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS learning_events"));
        assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS learning_code_proposals"));
        assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS outcome_contracts"));
        assert!(SCHEMA.contains("conversation_messages_fts"));
    }

    #[test]
    fn repairs_rebuild_transcript_fts() {
        assert!(
            DATA_REPAIRS
                .iter()
                .any(|stmt| stmt.contains("conversation_messages_fts") && stmt.contains("rebuild"))
        );
    }
}
