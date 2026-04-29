//! Workspace persistence trait.

use async_trait::async_trait;
use thinclaw_types::error::WorkspaceError;
use uuid::Uuid;

use crate::{MemoryChunk, MemoryDocument, SearchConfig, SearchResult, WorkspaceEntry};

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError>;
    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError>;
    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError>;
    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError>;
    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError>;
    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError>;
    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError>;

    /// Atomically replace all chunks for a document.
    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[(i32, String, Option<Vec<f32>>)],
    ) -> Result<(), WorkspaceError> {
        self.delete_chunks(document_id).await?;
        for (index, content, embedding) in chunks {
            self.insert_chunk(document_id, *index, content, embedding.as_deref())
                .await?;
        }
        Ok(())
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;
    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError>;
    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError>;
}
