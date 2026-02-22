-- Model Catalog Table
CREATE TABLE IF NOT EXISTS models_catalog (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    metadata TEXT NOT NULL, -- Stored as JSON string
    local_version TEXT,
    remote_version TEXT,
    last_checked_at INTEGER,
    status TEXT -- 'installed', 'outdated', 'unavailable'
);
