-- Durable, encrypted FIFO for cloud mutations. The envelope contains the
-- relative path, operation, and payload; no file metadata is stored in clear.
CREATE TABLE IF NOT EXISTS cloud_sync_outbox (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT NOT NULL UNIQUE,
    encrypted_envelope BLOB NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at_ms INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    last_error_code TEXT,
    lease_owner TEXT,
    lease_expires_at_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_cloud_sync_outbox_fifo
    ON cloud_sync_outbox(sequence, next_attempt_at_ms);

CREATE TABLE IF NOT EXISTS cloud_sync_quarantine (
    sequence INTEGER PRIMARY KEY,
    operation_id TEXT NOT NULL UNIQUE,
    encrypted_envelope BLOB NOT NULL,
    attempt_count INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    quarantined_at_ms INTEGER NOT NULL,
    reason_code TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cloud_sync_conflicts (
    sequence INTEGER PRIMARY KEY,
    operation_id TEXT NOT NULL UNIQUE,
    encrypted_envelope BLOB NOT NULL,
    attempt_count INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    conflicted_at_ms INTEGER NOT NULL,
    remote_generation INTEGER NOT NULL,
    reason_code TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cloud_sync_device_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    device_id TEXT NOT NULL UNIQUE,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0)
);
