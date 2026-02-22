-- Add chat_id to documents table for scoped RAG
ALTER TABLE documents ADD COLUMN chat_id TEXT;
CREATE INDEX IF NOT EXISTS idx_documents_chat_id ON documents(chat_id);
