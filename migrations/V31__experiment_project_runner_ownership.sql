-- Scope experiment projects and runner profiles to the principal that owns
-- them. Legacy rows are assigned to the sole campaign owner when that owner
-- can be inferred safely; unreferenced or ambiguously shared rows remain in
-- the fail-closed `default` namespace.

ALTER TABLE experiment_projects
    ADD COLUMN IF NOT EXISTS owner_user_id TEXT NOT NULL DEFAULT 'default';

ALTER TABLE experiment_runner_profiles
    ADD COLUMN IF NOT EXISTS owner_user_id TEXT NOT NULL DEFAULT 'default';

WITH inferred_project_owners AS (
    SELECT project_id, MIN(owner_user_id) AS owner_user_id
    FROM experiment_campaigns
    GROUP BY project_id
    HAVING COUNT(DISTINCT owner_user_id) = 1
)
UPDATE experiment_projects AS project
SET owner_user_id = inferred.owner_user_id
FROM inferred_project_owners AS inferred
WHERE project.id = inferred.project_id
  AND project.owner_user_id = 'default';

WITH inferred_runner_owners AS (
    SELECT runner_profile_id, MIN(owner_user_id) AS owner_user_id
    FROM experiment_campaigns
    GROUP BY runner_profile_id
    HAVING COUNT(DISTINCT owner_user_id) = 1
)
UPDATE experiment_runner_profiles AS runner
SET owner_user_id = inferred.owner_user_id
FROM inferred_runner_owners AS inferred
WHERE runner.id = inferred.runner_profile_id
  AND runner.owner_user_id = 'default';

CREATE INDEX IF NOT EXISTS idx_experiment_projects_owner_updated
    ON experiment_projects (owner_user_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_experiment_runner_profiles_owner_updated
    ON experiment_runner_profiles (owner_user_id, updated_at DESC);
