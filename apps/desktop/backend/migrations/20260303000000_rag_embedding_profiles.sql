-- Track which provider/model produced each derived vector. This prevents
-- equal-dimension model switches from silently mixing incompatible spaces.
ALTER TABLE chunks ADD COLUMN embedding_profile TEXT;

CREATE INDEX IF NOT EXISTS idx_chunks_embedding_profile
ON chunks(embedding_profile);

CREATE INDEX IF NOT EXISTS idx_documents_scoped_hash
ON documents(hash, project_id, chat_id);
