-- Track local encrypted secrets master-key versions independently of rows.
--
-- Rotation updates this table in the same transaction as secret re-encryption,
-- so new writes can use the active key_version instead of a hard-coded value.

CREATE TABLE IF NOT EXISTS secret_key_versions (
    version INTEGER PRIMARY KEY,
    status TEXT NOT NULL CHECK (status IN ('active', 'retired')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ
);

INSERT INTO secret_key_versions (version, status)
VALUES (1, 'active')
ON CONFLICT (version) DO NOTHING;

CREATE INDEX IF NOT EXISTS idx_secret_key_versions_status
    ON secret_key_versions(status);
