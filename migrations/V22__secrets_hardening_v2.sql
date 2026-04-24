-- Secrets hardening v2 metadata.
--
-- Existing rows are intentionally marked as encryption_version=1. The v2
-- decrypt path rejects them and requires credential re-entry; this avoids
-- pretending legacy ciphertext has AAD binding that it never had.

ALTER TABLE secrets
    ADD COLUMN IF NOT EXISTS encryption_version INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS key_version INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS cipher TEXT NOT NULL DEFAULT 'aes-256-gcm',
    ADD COLUMN IF NOT EXISTS kdf TEXT NOT NULL DEFAULT 'hkdf-sha256',
    ADD COLUMN IF NOT EXISTS aad_version INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS created_by TEXT,
    ADD COLUMN IF NOT EXISTS rotated_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_secrets_encryption_version
    ON secrets(encryption_version);

CREATE INDEX IF NOT EXISTS idx_secrets_key_version
    ON secrets(key_version);
