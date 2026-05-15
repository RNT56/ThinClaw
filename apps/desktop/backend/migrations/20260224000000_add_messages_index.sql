-- S-Perf #8: Add a composite index on messages(conversation_id, created_at)
-- Accelerates the primary pagination query:
--   SELECT * FROM messages WHERE conversation_id=? AND created_at<? ORDER BY created_at DESC LIMIT ?
CREATE INDEX IF NOT EXISTS idx_messages_conv_created
ON messages(conversation_id, created_at DESC);
