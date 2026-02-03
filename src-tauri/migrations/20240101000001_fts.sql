-- FTS5 table for chunks
-- We use 'content' option to avoid storing content twice (external content table)
-- but for simplicity and speed, sometimes creating a normal FTS table and syncing is easier/safer if rowids change or UUIDs are used.
-- Given 'id' is UUID (TEXT), mapping to FTS rowid (INT) is tricky with 'content=' option if we want to retrieve UUIDs easily.
-- Let's just create a normal FTS table and store the UUID in a column (unindexed for FTS but stored).
-- Or better: Use external content table, relying on SQLite's internal rowid which exists even for TEXT PK tables.

CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    content,
    chunk_uuid UNINDEXED, -- Store the UUID to retrieve it easily
    content='chunks',     -- External content
    content_rowid='rowid' -- Map FTS rowid to chunks.rowid
);

-- Triggers to keep synced
CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, content, chunk_uuid) VALUES (new.rowid, new.content, new.id);
END;

CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, chunk_uuid) VALUES('delete', old.rowid, old.content, old.id);
END;

CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, chunk_uuid) VALUES('delete', old.rowid, old.content, old.id);
  INSERT INTO chunks_fts(rowid, content, chunk_uuid) VALUES (new.rowid, new.content, new.id);
END;
