-- Cloud storage configuration and migration tracking
-- Migration: 20260301000000_cloud_storage

-- App-wide storage mode configuration
-- Keys: 'mode' ('local'|JSON), 'provider_config' (encrypted JSON),
--        'last_sync_at' (ms timestamp), 'manifest_key' (cloud path)
CREATE TABLE IF NOT EXISTS cloud_config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Insert default mode
INSERT OR IGNORE INTO cloud_config (key, value) VALUES ('mode', '"local"');

-- Migration history for auditing, resume, and rollback
CREATE TABLE IF NOT EXISTS cloud_migrations (
    id           TEXT PRIMARY KEY,
    direction    TEXT NOT NULL,       -- 'to_cloud' | 'to_local'
    provider     TEXT NOT NULL,       -- provider type slug
    started_at   INTEGER NOT NULL,    -- Unix ms
    completed_at INTEGER,            -- Unix ms, NULL if in-progress/failed
    files_total  INTEGER NOT NULL,
    files_done   INTEGER NOT NULL DEFAULT 0,
    bytes_total  INTEGER NOT NULL,
    bytes_done   INTEGER NOT NULL DEFAULT 0,
    status       TEXT NOT NULL DEFAULT 'in_progress',  -- 'in_progress'|'completed'|'failed'|'cancelled'
    error        TEXT                 -- NULL on success
);

-- Index for detecting interrupted migrations on launch
CREATE INDEX IF NOT EXISTS idx_cloud_migrations_status ON cloud_migrations(status);
