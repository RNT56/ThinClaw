-- Optional experiments / research subsystem.
--
-- Stores experiment projects, runner profiles, campaigns, trials,
-- artifact references, and lease-scoped remote execution state.

CREATE TABLE IF NOT EXISTS experiment_runner_profiles (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    backend JSONB NOT NULL,
    backend_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    image_or_runtime TEXT,
    gpu_requirements JSONB NOT NULL DEFAULT '{}'::jsonb,
    env_grants JSONB NOT NULL DEFAULT '{}'::jsonb,
    secret_references JSONB NOT NULL DEFAULT '[]'::jsonb,
    cache_policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    status JSONB NOT NULL DEFAULT '"draft"'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_experiment_runner_profiles_status
    ON experiment_runner_profiles (updated_at DESC);

CREATE TABLE IF NOT EXISTS experiment_projects (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    git_remote_name TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    preset JSONB NOT NULL DEFAULT '"autoresearch_single_file"'::jsonb,
    strategy_prompt TEXT NOT NULL DEFAULT '',
    workdir TEXT NOT NULL DEFAULT '.',
    prepare_command TEXT,
    run_command TEXT NOT NULL,
    mutable_paths JSONB NOT NULL DEFAULT '[]'::jsonb,
    fixed_paths JSONB NOT NULL DEFAULT '[]'::jsonb,
    primary_metric JSONB NOT NULL DEFAULT '{}'::jsonb,
    secondary_metrics JSONB NOT NULL DEFAULT '[]'::jsonb,
    comparison_policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    stop_policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    default_runner_profile_id UUID REFERENCES experiment_runner_profiles(id) ON DELETE SET NULL,
    promotion_mode TEXT NOT NULL DEFAULT 'branch_pr_draft',
    autonomy_mode JSONB NOT NULL DEFAULT '"autonomous"'::jsonb,
    status JSONB NOT NULL DEFAULT '"draft"'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_experiment_projects_status
    ON experiment_projects (updated_at DESC);

CREATE TABLE IF NOT EXISTS experiment_campaigns (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES experiment_projects(id) ON DELETE CASCADE,
    runner_profile_id UUID NOT NULL REFERENCES experiment_runner_profiles(id) ON DELETE RESTRICT,
    status JSONB NOT NULL DEFAULT '"pending_baseline"'::jsonb,
    baseline_commit TEXT,
    best_commit TEXT,
    best_metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    experiment_branch TEXT,
    remote_ref TEXT,
    worktree_path TEXT,
    started_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ,
    trial_count BIGINT NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    pause_reason TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_experiment_campaigns_project
    ON experiment_campaigns (project_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_experiment_campaigns_status
    ON experiment_campaigns (created_at DESC);

CREATE TABLE IF NOT EXISTS experiment_trials (
    id UUID PRIMARY KEY,
    campaign_id UUID NOT NULL REFERENCES experiment_campaigns(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    candidate_commit TEXT,
    parent_best_commit TEXT,
    status JSONB NOT NULL DEFAULT '"preparing"'::jsonb,
    runner_backend JSONB NOT NULL,
    exit_code INTEGER,
    metrics_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    summary TEXT,
    decision_reason TEXT,
    log_preview_path TEXT,
    artifact_manifest_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (campaign_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_experiment_trials_campaign
    ON experiment_trials (campaign_id, sequence ASC);

CREATE TABLE IF NOT EXISTS experiment_artifact_refs (
    id UUID PRIMARY KEY,
    trial_id UUID NOT NULL REFERENCES experiment_trials(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    uri_or_local_path TEXT NOT NULL,
    size_bytes BIGINT,
    fetchable BOOLEAN NOT NULL DEFAULT FALSE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_experiment_artifact_refs_trial
    ON experiment_artifact_refs (trial_id, created_at ASC);

CREATE TABLE IF NOT EXISTS experiment_leases (
    id UUID PRIMARY KEY,
    campaign_id UUID NOT NULL REFERENCES experiment_campaigns(id) ON DELETE CASCADE,
    trial_id UUID NOT NULL REFERENCES experiment_trials(id) ON DELETE CASCADE,
    runner_profile_id UUID NOT NULL REFERENCES experiment_runner_profiles(id) ON DELETE CASCADE,
    status JSONB NOT NULL DEFAULT '"pending"'::jsonb,
    token_hash TEXT NOT NULL,
    job_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    credentials_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    expires_at TIMESTAMPTZ NOT NULL,
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_experiment_leases_trial
    ON experiment_leases (trial_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_experiment_leases_expiry
    ON experiment_leases (expires_at);
