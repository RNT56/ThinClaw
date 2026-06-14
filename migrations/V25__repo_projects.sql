-- Durable GitHub repo project supervisor state.

CREATE TABLE IF NOT EXISTS repo_projects (
    id UUID PRIMARY KEY,
    state TEXT NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_repo_projects_state_updated
    ON repo_projects(state, updated_at DESC);

CREATE TABLE IF NOT EXISTS repo_project_repos (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES repo_projects(id) ON DELETE CASCADE,
    owner TEXT NOT NULL,
    repo TEXT NOT NULL,
    enrolled BOOLEAN NOT NULL DEFAULT TRUE,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_id, owner, repo)
);

CREATE INDEX IF NOT EXISTS idx_repo_project_repos_project
    ON repo_project_repos(project_id, owner, repo);

CREATE TABLE IF NOT EXISTS repo_project_tasks (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES repo_projects(id) ON DELETE CASCADE,
    repo_id UUID NOT NULL REFERENCES repo_project_repos(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_repo_project_tasks_project_state
    ON repo_project_tasks(project_id, state, priority DESC, updated_at DESC);

CREATE TABLE IF NOT EXISTS repo_worker_runs (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES repo_projects(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES repo_project_tasks(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_repo_worker_runs_project_state
    ON repo_worker_runs(project_id, state, updated_at DESC);

CREATE TABLE IF NOT EXISTS repo_project_events (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES repo_projects(id) ON DELETE CASCADE,
    task_id UUID REFERENCES repo_project_tasks(id) ON DELETE SET NULL,
    event_kind TEXT NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_repo_project_events_project_created
    ON repo_project_events(project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS repo_merge_gate_decisions (
    project_id UUID NOT NULL REFERENCES repo_projects(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES repo_project_tasks(id) ON DELETE CASCADE,
    data JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(project_id, task_id)
);
